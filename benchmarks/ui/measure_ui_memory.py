#!/usr/bin/env python3
"""Measure the graphical editor on the 16K photograph.

The roadmap's memory target is **64 MB of CPU working memory for 16K
localized editing**. This script measures what the editor costs when it
shows a rectangle of the 16K photograph, changes a rectangle of it and
saves it.

It measures the **peak working set** of the editor's own process, taken
from the operating system's `GetProcessMemoryInfo` while the process is
alive. The figure is the process's high-water mark: the operating system,
the loader, the binary's own text and data, the Rust heap, the editor's
decoded-tile cache, the display buffer and every other buffer are all
inside it.

It also measures how long the first viewport takes and how long a pan
takes — a pan being a second, different rectangle, which is what the
viewer engine's plan is for.

Run from the repository root:

    python benchmarks/ui/measure_ui_memory.py
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

sys.path.insert(0, str(REPO / "benchmarks"))
from measure_peak import peak_working_set  # noqa: E402

sys.path.insert(0, str(HERE))
from ui_client import Editor  # noqa: E402

#: The corpus image the measurement uses: the 16K photograph.
IMAGE_ID = "user-photo-15360x8640"

#: The memory target: 64 MB.
TARGET_BYTES = 64 * 1024 * 1024

#: The viewport the measurement shows: 1024x1024, as the preview
#: benchmarks used.
VIEWPORT = 1024


def cache_index(cache_root: Path) -> dict[str, Path]:
    """Map every cached canonical digest to its raw file."""
    index: dict[str, Path] = {}
    if not cache_root.exists():
        return index
    for sidecar in cache_root.glob("*.json"):
        if sidecar.name == "index.json":
            continue
        try:
            record = json.loads(sidecar.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            continue
        digest = record.get("canonical_sha256")
        name = record.get("canonical_path")
        if digest and name:
            raw = cache_root / name
            if raw.exists():
                index[digest] = raw
    return index


def start_editor(vpng: Path, picture: Path) -> tuple[Editor, subprocess.Popen]:
    """Start the editor over a file and read the address it prints."""
    process = subprocess.Popen(
        [str(vpng), "ui", str(picture), "--no-browser"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    address = ""
    for line in process.stdout:
        stripped = line.strip()
        if stripped.startswith("vpng: the editor is at "):
            address = stripped[len("vpng: the editor is at ") :]
            break
    if not address:
        process.kill()
        process.wait()
        raise RuntimeError("the editor printed no address")
    base, _, token = address.partition("?token=")
    return Editor(base, token), process


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=REPO / "benchmarks/corpus_manifest.json")
    parser.add_argument("--cache", type=Path, default=REPO / "corpus-data/cache")
    parser.add_argument("--vpng", type=Path, default=REPO / "target/release/vpng.exe")
    parser.add_argument("--work", type=Path, default=REPO / "benchmarks/work/ui-memory")
    parser.add_argument("--output", type=Path, default=REPO / "benchmarks/p14_ui_memory_results.json")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--runs", type=int, default=5)
    args = parser.parse_args()

    if not args.vpng.exists():
        print(f"no vpng binary at {args.vpng}; run `cargo build --release -p vpng-cli`")
        return 2

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    entry = next((item for item in manifest["images"] if item["id"] == IMAGE_ID), None)
    if entry is None:
        print(f"the corpus does not hold {IMAGE_ID}")
        return 2

    raw = cache_index(args.cache).get(entry["sha256"])
    if raw is None:
        print(f"no cached canonical pixels for {IMAGE_ID}")
        return 2

    width, height, fmt = entry["width"], entry["height"], entry["pixel_format"]
    args.work.mkdir(parents=True, exist_ok=True)
    picture = args.work / "picture.vpng"

    encode = subprocess.run(
        [
            str(args.vpng), "encode", str(raw), str(picture),
            "--input-format", "raw",
            "--format", fmt,
            "--width", str(width),
            "--height", str(height),
            "--tile-size", str(args.tile_size),
            "--workers", str(args.workers),
        ],
        capture_output=True,
        text=True,
    )
    if encode.returncode != 0:
        print(f"the encode failed: {encode.stdout}{encode.stderr}")
        return 2
    file_bytes = picture.stat().st_size

    first_viewport_ms: list[float] = []
    pan_ms: list[float] = []
    pick_ms: list[float] = []
    save_ms: list[float] = []
    peaks: list[int] = []
    cache_bytes: list[int] = []
    history_bytes: list[int] = []

    for run in range(args.runs):
        editor, process = start_editor(args.vpng, picture)
        try:
            state = editor.state()
            baseline = peak_working_set(process.pid)

            pixels, elapsed = editor.region(0, 0, VIEWPORT, VIEWPORT)
            assert len(pixels) == VIEWPORT * VIEWPORT * 4, "the viewport is the wrong size"
            first_viewport_ms.append(elapsed)
            after_first = peak_working_set(process.pid)

            # A pan: a different rectangle, in the middle of the picture.
            _, elapsed = editor.region(
                width // 2 - VIEWPORT // 2, height // 2 - VIEWPORT // 2,
                VIEWPORT, VIEWPORT,
            )
            pan_ms.append(elapsed)
            after_pan = peak_working_set(process.pid)

            _, elapsed = editor.pick(width // 2, height // 2)
            pick_ms.append(elapsed)

            # A localized edit: a 256x256 rectangle, which is one tile.
            editor.colour(200, 100, 50, 255)
            editor.fill(width // 2, height // 2, 256, 256)
            after_edit = peak_working_set(process.pid)

            saved, elapsed = editor.save()
            save_ms.append(elapsed)
            after_save = peak_working_set(process.pid)

            final_state = editor.state()
            cache_bytes.append(final_state["cache_current_bytes"])
            history_bytes.append(final_state["history_current_bytes"])

            peaks.append(max(after_first, after_pan, after_edit, after_save))
            print(
                f"  run {run + 1}: baseline {baseline:,} B, "
                f"first viewport {first_viewport_ms[-1]:.1f} ms, "
                f"peak {peaks[-1]:,} B, "
                f"{saved['tiles_rewritten']} tiles rewritten, "
                f"{saved['tiles_copied']} copied"
            )
            state = final_state
        finally:
            process.kill()
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                pass
            if process.stdout:
                process.stdout.close()
            if process.stderr:
                process.stderr.close()

        # The save rewrote the file, so the next run starts from the edited
        # picture. The edit is undone by the engine's history only, so the
        # file is rebuilt for every run.
        subprocess.run(
            [
                str(args.vpng), "encode", str(raw), str(picture),
                "--input-format", "raw",
                "--format", fmt,
                "--width", str(width),
                "--height", str(height),
                "--tile-size", str(args.tile_size),
                "--workers", str(args.workers),
            ],
            capture_output=True,
            text=True,
            check=True,
        )

    def summary(values: list[float]) -> dict:
        ordered = sorted(values)
        middle = len(ordered) // 2
        if len(ordered) % 2:
            median = float(ordered[middle])
        else:
            median = (ordered[middle - 1] + ordered[middle]) / 2.0
        return {
            "count": len(values),
            "min": round(min(values), 3),
            "max": round(max(values), 3),
            "median": round(median, 3),
            "mean": round(sum(values) / len(values), 3),
        }

    payload = {
        "schema": "vpng.ui.memory.v1",
        "generated_by": "benchmarks/ui/measure_ui_memory.py",
        "image": {
            "id": IMAGE_ID,
            "width": width,
            "height": height,
            "pixel_format": fmt,
            "canonical_bytes": entry["canonical_bytes"],
            "tile_size": args.tile_size,
        },
        "file_bytes": file_bytes,
        "viewport": {
            "width": VIEWPORT,
            "height": VIEWPORT,
            "pixels": VIEWPORT * VIEWPORT,
            "rgba_bytes": VIEWPORT * VIEWPORT * 4,
        },
        "runs": args.runs,
        "scope": {
            "measured": "the editor process's peak working set, from the operating "
            "system's GetProcessMemoryInfo, taken while the process was alive",
            "included": "the operating system, the loader, the binary's text and data, "
            "the Rust heap, the decoded-tile cache, the display buffer, the undo "
            "history and every other buffer the editor allocated",
            "excluded": "nothing: the figure is the whole process",
            "target_bytes": TARGET_BYTES,
            "note": "the figure is a whole-process number; the editor's own buffers "
            "are reported separately so the two can be told apart",
        },
        "peak_working_set_bytes": {
            "min": min(peaks),
            "max": max(peaks),
            "median": sorted(peaks)[len(peaks) // 2],
            "per_run": peaks,
        },
        "editor_buffers_bytes": {
            "cache_current": max(cache_bytes),
            "history_current": max(history_bytes),
        },
        "timings_ms": {
            "first_viewport": summary(first_viewport_ms),
            "pan": summary(pan_ms),
            "pick": summary(pick_ms),
            "save": summary(save_ms),
        },
        "tile_count": state["tile_count"],
        "visible_tiles": state["visible_tiles"],
        "prefetch_tiles": state["prefetch_tiles"],
        "max_region_pixels": state["max_region_pixels"],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=1), encoding="utf-8")
    print(f"wrote {args.output} ({args.output.stat().st_size} bytes)")
    peak = payload["peak_working_set_bytes"]["max"]
    print(
        f"  peak working set {peak:,} B against the {TARGET_BYTES:,} B target "
        f"({peak / TARGET_BYTES * 100:.1f}%)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
