#!/usr/bin/env python3
"""Measure the Python editing path's memory on a 16K picture.

The roadmap's memory target is: **≤64 MB of CPU working memory for a
localized edit of a 16K picture**. This script measures that path through
the Python extension, in a **fresh process**, so the figure is the edit's
own and is not contaminated by an earlier encode in the same process.

The run is two phases:

1. The parent encodes the 16K picture with the Python API, so the file the
   edit works on is a real VPNG file.
2. A **child process** opens that file, reads a region, writes a region and
   commits, and reports its own peak working set.

**Included** in the figure: the whole child process — the CPython
interpreter, the extension module, the decoded-tile cache, the undo history,
the tile payloads in flight, the rewritten payloads waiting to be written,
the container metadata, the stacks and the allocator's own bookkeeping.
**Excluded**: the `.vpng` file on disk, the operating system's page cache,
and the memory of other processes.

Run from the repository root:

    python benchmarks/python/run_python_memory.py

The raw results are written to `benchmarks/p9_python_memory_results.json`.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "python"))
sys.path.insert(0, str(HERE))

from run_python_bench import cache_index, peak_working_set_bytes  # noqa: E402

#: The memory target from the roadmap: 64 MB of CPU working memory.
TARGET_BYTES = 64 * 1024 * 1024

#: The picture the target is stated against.
TARGET_ID = "user-photo-15360x8640"


def find_target(cache_root: Path, manifest_path: Path) -> tuple[dict, Path]:
    """The 16K picture and its cached canonical pixels."""
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    cache = cache_index(cache_root)
    for entry in manifest["images"]:
        if entry["id"] == TARGET_ID:
            raw = cache.get(entry["sha256"])
            if raw is None:
                raise SystemExit(f"no cached canonical pixels for {TARGET_ID}")
            return entry, raw
    raise SystemExit(f"{TARGET_ID} is not in the manifest")


def child_main(target: str) -> int:
    """The phase that is measured: open, read a region, write it, commit."""
    import vpng as vpng_py

    path = Path(target)
    started = time.perf_counter()
    with vpng_py.open_document(path) as doc:
        width = doc.width
        height = doc.height
        region = doc.read_region(x=0, y=0, width=1024, height=1024)
        pixels = bytes(region)
        with doc.edit() as tx:
            # Change one pixel inside the region, so one tile is rewritten.
            changed = bytearray(pixels)
            changed[0] = (changed[0] + 1) & 0xFF
            tx.write_region(bytes(changed), x=0, y=0, width=1024, height=1024)
            report = tx.commit(path)
    elapsed = (time.perf_counter() - started) * 1000.0

    payload = {
        "width": width,
        "height": height,
        "elapsed_ms": round(elapsed, 3),
        "tiles_rewritten": report.tiles_rewritten,
        "tiles_copied": report.tiles_copied,
        "copied_payload_bytes": report.copied_payload_bytes,
        "rewritten_payload_bytes": report.rewritten_payload_bytes,
        "peak_working_set_bytes": peak_working_set_bytes(),
    }
    print("CHILD_RESULT " + json.dumps(payload))
    return 0


def main() -> int:
    # The child mode is handled before the parser runs: `--child` is an
    # internal switch, not a public option.
    if "--child" in sys.argv:
        target = sys.argv[sys.argv.index("--child") + 1]
        return child_main(target)

    parser = argparse.ArgumentParser()
    parser.add_argument("--cache-root", default=str(REPO / "corpus-data" / "cache"))
    parser.add_argument(
        "--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json")
    )
    parser.add_argument(
        "--output", default=str(REPO / "benchmarks" / "p9_python_memory_results.json")
    )
    parser.add_argument("--work", default=str(REPO / "benchmarks" / "work" / "python"))
    args = parser.parse_args()

    import vpng as vpng_py

    entry, raw = find_target(Path(args.cache_root), Path(args.manifest))
    width, height = entry["width"], entry["height"]
    fmt = entry["pixel_format"]
    print(f"target: {entry['id']} {width}x{height} {fmt}")

    work = Path(args.work)
    work.mkdir(parents=True, exist_ok=True)
    target_file = work / "memory-target.vpng"

    pixels = raw.read_bytes()
    encode_started = time.perf_counter()
    vpng_py.encode_pixels(pixels, width, height, target_file, format=fmt)
    encode_ms = (time.perf_counter() - encode_started) * 1000.0
    print(f"encoded: {target_file.stat().st_size} bytes in {encode_ms:.1f} ms")

    # The child runs in a fresh process, so the figure is the edit's own.
    proc = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), "--child", str(target_file)],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stdout)
        print(proc.stderr, file=sys.stderr)
        return proc.returncode

    line = next(
        line for line in proc.stdout.splitlines() if line.startswith("CHILD_RESULT ")
    )
    child = json.loads(line[len("CHILD_RESULT ") :])

    peak = child["peak_working_set_bytes"]
    within = None if peak is None else peak <= TARGET_BYTES

    payload = {
        "generated": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "target": {
            "id": entry["id"],
            "width": width,
            "height": height,
            "pixel_format": fmt,
            "canonical_bytes": entry["canonical_bytes"],
            "file_bytes": target_file.stat().st_size,
        },
        "encode_ms": round(encode_ms, 3),
        "edit": child,
        "target_bytes": TARGET_BYTES,
        "peak_working_set_bytes": peak,
        "within_target": within,
        "memory_scope": {
            "included": (
                "the whole child process: the CPython interpreter, the extension "
                "module, the decoded-tile cache, the undo history, the tile "
                "payloads in flight, the rewritten payloads waiting to be written, "
                "the container metadata, the stacks and the allocator's bookkeeping"
            ),
            "excluded": (
                "the .vpng file on disk, the operating system's page cache, and the "
                "memory of other processes"
            ),
        },
    }
    Path(args.output).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    print()
    print(f"edit elapsed:          {child['elapsed_ms']} ms")
    print(f"tiles rewritten:       {child['tiles_rewritten']}")
    print(f"tiles copied:          {child['tiles_copied']}")
    print(f"peak working set:      {peak}")
    print(f"target ({TARGET_BYTES}): {within}")
    print(f"written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
