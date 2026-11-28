#!/usr/bin/env python3
"""Measure the graphical editor over the 750-image corpus.

The editor is a **window** over the editing engine: it opens a file, shows
a rectangle of it, changes a rectangle of it and saves it. This script
measures the editor the way a person would use it — it starts the real
`vpng ui` program, talks to it over HTTP and checks what comes back — and
it records the things the editor actually promises:

* **The editor shows exactly what the file holds.** The rectangle the
  editor returns is compared, byte for byte, with the canonical pixels the
  corpus manifest records. The comparison is made against an independent
  implementation of the display conversion, so a bug in the editor cannot
  hide behind a matching bug in the checker.
* **A pick is exact.** The pixel the editor names is the pixel the file
  holds.
* **A fill changes exactly the rectangle that was asked for**, and an undo
  puts it back.
* **A save rewrites only what changed.** Every tile the edit did not touch
  keeps its payload byte for byte, and the saved file still passes
  `vpng check`.
* **The editor's own claims are recorded**: the number of tiles it plans
  to show, the number it prefetches, and how long each call took.

Nothing here is a claim about speed on its own. The timings are reported
so the report can say what was measured; they are not a performance claim
about the editor.

Run from the repository root:

    python benchmarks/ui/run_ui_corpus.py
    python benchmarks/ui/run_ui_corpus.py --sample 50 --jobs 1
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

sys.path.insert(0, str(HERE))
from ui_client import (  # noqa: E402
    Editor,
    EditorError,
    expected_rect,
    read_container,
    rgba8_of,
)

#: The colour the measurement fills with.
FILL_COLOUR = (17, 34, 51, 255)

#: The size of the rectangle the measurement fills.
FILL_SIZE = 16


def fill_as_the_file_holds(colour: tuple[int, int, int, int], pixel_format: str) -> bytes:
    """The colour the editor writes into a file of this format.

    A **grey** file cannot hold a colour, so the editor takes the mean of
    the colour's three channels, rounded half up — the rule the ADR states.
    A file with **no alpha channel** ignores the colour's alpha. The
    measurement compares against this, so a grey file is checked against
    the grey the editor actually wrote rather than against a colour it
    cannot hold.
    """
    red, green, blue, alpha = colour
    grey = (red + green + blue + 1) // 3
    if pixel_format == "gray8":
        return bytes([grey, grey, grey, 255])
    if pixel_format == "graya8":
        return bytes([grey, grey, grey, alpha])
    if pixel_format == "rgb8":
        return bytes([red, green, blue, 255])
    if pixel_format == "rgba8":
        return bytes([red, green, blue, alpha])
    raise ValueError(f"the editor does not edit a {pixel_format} picture")


def median(values: list[float]) -> float | None:
    """The middle sample, or the mean of the two middle samples."""
    if not values:
        return None
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return float(ordered[middle])
    return (ordered[middle - 1] + ordered[middle]) / 2.0


def p95(values: list[float]) -> float | None:
    """The nearest-rank p95."""
    if not values:
        return None
    ordered = sorted(values)
    rank = math.ceil(0.95 * len(ordered))
    return float(ordered[max(rank - 1, 0)])


def stdev(values: list[float]) -> float | None:
    """The sample standard deviation (n-1)."""
    if len(values) < 2:
        return None
    return statistics.stdev(values)


def distribution(values: list[float]) -> dict:
    """The distribution summary of a list of measurements."""
    if not values:
        return {"count": 0}
    return {
        "count": len(values),
        "min": round(min(values), 6),
        "max": round(max(values), 6),
        "mean": round(statistics.fmean(values), 6),
        "median": round(median(values), 6),
        "p95": round(p95(values), 6),
        "stdev": round(stdev(values), 6) if stdev(values) is not None else None,
    }


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


def sha256_bytes(data: bytes) -> str:
    """The SHA-256 of some bytes."""
    return hashlib.sha256(data).hexdigest()


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
        if not stripped:
            continue
    if not address:
        process.kill()
        process.wait()
        raise RuntimeError("the editor printed no address")

    base, _, token = address.partition("?token=")
    return Editor(base, token), process


def stop_editor(process: subprocess.Popen) -> None:
    """Stop the editor."""
    process.kill()
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        pass
    if process.stdout:
        process.stdout.close()
    if process.stderr:
        process.stderr.close()


def measure_one(
    entry: dict,
    raw_path: Path,
    vpng: Path,
    work: Path,
    tile_size: int,
    workers: int,
) -> dict:
    """Drive the editor over one picture and record what it did."""
    width = entry["width"]
    height = entry["height"]
    fmt = entry["pixel_format"]
    image_id = entry["id"]

    picture = work / f"{image_id}.vpng"
    encode = subprocess.run(
        [
            str(vpng), "encode", str(raw_path), str(picture),
            "--input-format", "raw",
            "--format", fmt,
            "--width", str(width),
            "--height", str(height),
            "--tile-size", str(tile_size),
            "--workers", str(workers),
        ],
        capture_output=True,
        text=True,
    )
    if encode.returncode != 0:
        raise RuntimeError(
            f"the encode of {image_id} failed (exit {encode.returncode}): "
            f"{encode.stdout}{encode.stderr}"
        )

    original = picture.read_bytes()
    original_container = read_container(picture)

    result: dict = {
        "id": image_id,
        "category": entry["category"],
        "width": width,
        "height": height,
        "pixel_format": fmt,
        "file_bytes": len(original),
        "tile_count": original_container.tile_count,
        "errors": [],
    }

    editor, process = start_editor(vpng, picture)
    try:
        # 1. The state: the editor must report the file's own geometry.
        state_started = time.perf_counter()
        state = editor.state()
        result["state_ms"] = round((time.perf_counter() - state_started) * 1000, 3)
        result["state_width"] = state["width"]
        result["state_height"] = state["height"]
        result["state_format"] = state["format"]
        result["state_tile_count"] = state["tile_count"]
        if state["width"] != width or state["height"] != height:
            result["errors"].append("the editor reports the wrong geometry")
        if state["format"] != fmt:
            result["errors"].append("the editor reports the wrong format")

        # 2. A viewport: the pixels must be exactly the canonical pixels.
        view_width = min(width, 512)
        view_height = min(height, 512)
        pixels, region_ms = editor.region(0, 0, view_width, view_height)
        result["region_ms"] = round(region_ms, 3)
        result["region_bytes"] = len(pixels)

        raw = raw_path.read_bytes()
        expected = rgba8_of(raw, width, height, fmt)
        wanted = expected_rect(expected, width, 0, 0, view_width, view_height)
        result["region_exact"] = pixels == wanted
        if not result["region_exact"]:
            result["errors"].append("the viewport is not the canonical pixels")

        # The viewer engine's plan is made when the viewport is read, so the
        # state is read again afterwards.
        after_view = editor.state()
        result["visible_tiles"] = after_view["visible_tiles"]
        result["prefetch_tiles"] = after_view["prefetch_tiles"]
        result["prefetched_tiles"] = after_view["prefetched_tiles"]

        # 3. A pick: the pixel must be exactly the pixel the file holds.
        pick_x = min(width - 1, view_width - 1)
        pick_y = min(height - 1, view_height - 1)
        picked, pick_ms = editor.pick(pick_x, pick_y)
        result["pick_ms"] = round(pick_ms, 3)
        base = (pick_y * width + pick_x) * 4
        result["pick_exact"] = (
            picked["r"] == expected[base]
            and picked["g"] == expected[base + 1]
            and picked["b"] == expected[base + 2]
            and picked["a"] == expected[base + 3]
        )
        if not result["pick_exact"]:
            result["errors"].append("the pick is not the pixel the file holds")

        # 4. A fill: exactly the rectangle, and nothing else.
        fill_x = min(width - 1, max(0, width // 2))
        fill_y = min(height - 1, max(0, height // 2))
        fill_width = min(FILL_SIZE, width - fill_x)
        fill_height = min(FILL_SIZE, height - fill_y)
        editor.colour(*FILL_COLOUR)
        report, fill_ms = editor.fill(fill_x, fill_y, fill_width, fill_height)
        result["fill_ms"] = round(fill_ms, 3)
        result["fill_pixels"] = report["pixels"]
        result["fill_dirty_tiles"] = report["dirty_tiles"]

        after, _ = editor.region(fill_x, fill_y, fill_width, fill_height)
        wanted_fill = fill_as_the_file_holds(FILL_COLOUR, fmt) * (fill_width * fill_height)
        result["fill_exact"] = after == wanted_fill
        if not result["fill_exact"]:
            result["errors"].append("the fill is not the colour that was chosen")

        # 5. An undo puts the pixels back.
        editor.undo()
        restored, _ = editor.region(fill_x, fill_y, fill_width, fill_height)
        wanted_restored = expected_rect(
            expected, width, fill_x, fill_y, fill_width, fill_height
        )
        result["undo_exact"] = restored == wanted_restored
        if not result["undo_exact"]:
            result["errors"].append("the undo did not put the pixels back")

        # 6. The fill again, and a save.
        editor.fill(fill_x, fill_y, fill_width, fill_height)
        saved, save_ms = editor.save()
        result["save_ms"] = round(save_ms, 3)
        result["tiles_rewritten"] = saved["tiles_rewritten"]
        result["tiles_copied"] = saved["tiles_copied"]
        result["copied_payload_bytes"] = saved["copied_payload_bytes"]
        result["rewritten_payload_bytes"] = saved["rewritten_payload_bytes"]
        result["saved_file_bytes"] = saved["file_size"]

        # 7. The saved file must still be a valid container.
        check = subprocess.run(
            [str(vpng), "check", str(picture)], capture_output=True, text=True
        )
        result["check_exit"] = check.returncode
        if check.returncode != 0:
            result["errors"].append("the saved file does not pass `vpng check`")

        # 8. The tiles the edit did not touch keep their payload bytes.
        saved_container = read_container(picture)
        original_payloads = original_container.payloads()
        saved_payloads = saved_container.payloads()
        result["saved_tile_count"] = saved_container.tile_count
        untouched = 0
        changed = 0
        for before, after_bytes in zip(original_payloads, saved_payloads):
            if before == after_bytes:
                untouched += 1
            else:
                changed += 1
        result["payloads_untouched"] = untouched
        result["payloads_changed"] = changed
        # The engine's own report and the bytes on disk must agree: the tiles
        # it says it rewrote are the tiles whose payload changed, and the
        # tiles it says it copied are the tiles whose payload is byte for
        # byte what it was.
        accounted = (
            changed == result["tiles_rewritten"]
            and untouched == result["tiles_copied"]
            and changed + untouched == result["tile_count"]
        )
        result["untouched_payloads_identical"] = accounted
        if not accounted:
            result["errors"].append(
                "the rewritten and copied tiles do not account for every tile"
            )

        # 9. The saved file's pixels: the fill is there, and nothing else
        #    moved.
        saved_raw = work / f"{image_id}.saved.raw"
        decode = subprocess.run(
            [
                str(vpng), "decode", str(picture), str(saved_raw),
                "--format", "raw",
            ],
            capture_output=True,
            text=True,
        )
        if decode.returncode != 0:
            result["errors"].append("the saved file does not decode")
        else:
            saved_pixels = saved_raw.read_bytes()
            saved_rgba = rgba8_of(saved_pixels, width, height, fmt)
            fill_area = expected_rect(
                saved_rgba, width, fill_x, fill_y, fill_width, fill_height
            )
            result["saved_fill_exact"] = (
                len(fill_area) == fill_width * fill_height * 4
                and fill_area
                == fill_as_the_file_holds(FILL_COLOUR, fmt) * (fill_width * fill_height)
            )
            if not result["saved_fill_exact"]:
                result["errors"].append("the saved file does not hold the fill")
            # Everything outside the filled rows is untouched.
            outside_equal = True
            for row in range(height):
                if fill_y <= row < fill_y + fill_height:
                    continue
                start = (row * width) * 4
                if saved_rgba[start : start + width * 4] != expected[
                    start : start + width * 4
                ]:
                    outside_equal = False
                    break
            result["saved_outside_exact"] = outside_equal
            if not outside_equal:
                result["errors"].append("the saved file changed a row it should not")
            saved_raw.unlink(missing_ok=True)
    finally:
        stop_editor(process)

    result["ok"] = not result["errors"]
    result["sha256"] = sha256_bytes(original)
    picture.unlink(missing_ok=True)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=REPO / "benchmarks/corpus_manifest.json")
    parser.add_argument("--cache", type=Path, default=REPO / "corpus-data/cache")
    parser.add_argument("--vpng", type=Path, default=REPO / "target/release/vpng.exe")
    parser.add_argument("--work", type=Path, default=REPO / "benchmarks/work/ui")
    parser.add_argument("--output", type=Path, default=REPO / "benchmarks/p14_ui_results.json")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--sample", type=int, default=0, help="measure only the first N images")
    parser.add_argument("--category", default="", help="measure only one category")
    args = parser.parse_args()

    if not args.vpng.exists():
        print(f"no vpng binary at {args.vpng}; run `cargo build --release -p vpng-cli`")
        return 2

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    images = manifest["images"]
    if args.category:
        images = [item for item in images if item["category"] == args.category]
    if args.sample:
        images = images[: args.sample]

    index = cache_index(args.cache)
    work = args.work
    work.mkdir(parents=True, exist_ok=True)

    results: list[dict] = []
    started = time.perf_counter()
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = []
        for entry in images:
            raw = index.get(entry["sha256"])
            if raw is None:
                print(f"  no cached canonical pixels for {entry['id']}; skipping")
                continue
            futures.append(
                pool.submit(
                    measure_one,
                    entry,
                    raw,
                    args.vpng,
                    work,
                    args.tile_size,
                    args.workers,
                )
            )
        for future in futures:
            results.append(future.result())
            print(f"  {results[-1]['id']}: {'ok' if results[-1]['ok'] else results[-1]['errors']}")

    elapsed = time.perf_counter() - started

    by_category: dict[str, list[dict]] = {}
    for row in results:
        by_category.setdefault(row["category"], []).append(row)

    def summarise(rows: list[dict]) -> dict:
        return {
            "images": len(rows),
            "ok": sum(1 for row in rows if row["ok"]),
            "failed": sum(1 for row in rows if not row["ok"]),
            "region_exact": sum(1 for row in rows if row.get("region_exact")),
            "pick_exact": sum(1 for row in rows if row.get("pick_exact")),
            "fill_exact": sum(1 for row in rows if row.get("fill_exact")),
            "undo_exact": sum(1 for row in rows if row.get("undo_exact")),
            "saved_fill_exact": sum(1 for row in rows if row.get("saved_fill_exact")),
            "saved_outside_exact": sum(1 for row in rows if row.get("saved_outside_exact")),
            "check_failures": sum(1 for row in rows if row.get("check_exit", 1) != 0),
            "untouched_payloads_identical": sum(
                1 for row in rows if row.get("untouched_payloads_identical")
            ),
            "visible_tiles_total": sum(row.get("visible_tiles", 0) for row in rows),
            "prefetch_tiles_total": sum(row.get("prefetch_tiles", 0) for row in rows),
            "prefetched_tiles_total": sum(row.get("prefetched_tiles", 0) for row in rows),
            "tiles_rewritten_total": sum(row.get("tiles_rewritten", 0) for row in rows),
            "tiles_copied_total": sum(row.get("tiles_copied", 0) for row in rows),
            "copied_payload_bytes_total": sum(
                row.get("copied_payload_bytes", 0) for row in rows
            ),
            "rewritten_payload_bytes_total": sum(
                row.get("rewritten_payload_bytes", 0) for row in rows
            ),
            "region_ms": distribution([row["region_ms"] for row in rows if "region_ms" in row]),
            "pick_ms": distribution([row["pick_ms"] for row in rows if "pick_ms" in row]),
            "fill_ms": distribution([row["fill_ms"] for row in rows if "fill_ms" in row]),
            "save_ms": distribution([row["save_ms"] for row in rows if "save_ms" in row]),
            "state_ms": distribution([row["state_ms"] for row in rows if "state_ms" in row]),
        }

    payload = {
        "schema": "vpng.ui.corpus.v1",
        "generated_by": "benchmarks/ui/run_ui_corpus.py",
        "host": {
            "platform": sys.platform,
            "python": sys.version.split()[0],
        },
        "method": {
            "editor": "the real `vpng ui` program, driven over HTTP",
            "client": "the standard library's urllib; no third-party package",
            "checker": "an independent reader of the container, written from the "
            "specification rather than from the project's own decoder",
            "tile_size": args.tile_size,
            "encoder_workers": args.workers,
            "viewport": "the top-left 512x512 rectangle, or the whole picture "
            "when it is smaller",
            "fill": {
                "colour": list(FILL_COLOUR),
                "size": FILL_SIZE,
                "position": "the middle of the picture",
            },
            "scope": "the timings are the wall clock of one HTTP round trip on "
            "this host; they are not a performance claim and they include the "
            "Python client",
        },
        "corpus": {
            "total_images": manifest["total_images"],
            "measured": len(results),
            "categories": sorted(by_category),
            "elapsed_seconds": round(elapsed, 3),
        },
        "overall": summarise(results),
        "by_category": {
            name: summarise(rows) for name, rows in sorted(by_category.items())
        },
        "images": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=1), encoding="utf-8")
    print(f"wrote {args.output} ({args.output.stat().st_size} bytes)")
    print(
        f"  {payload['overall']['ok']}/{payload['overall']['images']} images passed; "
        f"{payload['overall']['failed']} failed"
    )
    return 0 if payload["overall"]["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
