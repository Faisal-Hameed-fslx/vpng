#!/usr/bin/env python3
"""Measure what the layer commands cost in memory, on the 16K photograph.

The layer model is about *structure*, not about small buffers: a composite
blends the whole canvas, so it holds the whole canvas. This script measures
that, exactly, so the milestone report can state the cost instead of leaving
it to inference.

It measures the **peak working set** of each command, taken from the
operating system's own `GetProcessMemoryInfo` while the process is alive.
The figure is the process's high-water mark: the operating system, the
loader, the binary's own text and data, the Rust heap and every buffer the
command allocated are all inside it.

Run from the repository root:

    python benchmarks/layers/measure_layer_memory.py
"""

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

#: The corpus image the measurement uses: the 16K photograph.
IMAGE_ID = "user-photo-15360x8640"


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


def measure(command: list[str]) -> dict:
    """Run a command and report its peak working set and its time."""
    started = time.perf_counter()
    process = subprocess.Popen(
        command, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    peak = 0
    while process.poll() is None:
        peak = max(peak, peak_working_set(process.pid))
    process.wait()
    peak = max(peak, peak_working_set(process.pid))
    return {
        "command": command[1:],
        "exit": process.returncode,
        "peak_working_set_bytes": peak,
        "elapsed_ms": round((time.perf_counter() - started) * 1000),
    }


def write_overlay(source: Path, target: Path, width: int, height: int, fmt: str) -> None:
    """Write an overlay: the top-left quadrant, colour-inverted and opaque."""
    channels = {"gray8": (1, 1), "graya8": (2, 1), "rgb8": (3, 3), "rgba8": (4, 3)}
    bpp, colour = channels[fmt]
    half_width = max(1, (width + 1) // 2)
    half_height = max(1, (height + 1) // 2)
    row_bytes = width * bpp
    crop_bytes = half_width * bpp

    with source.open("rb") as reader, target.open("wb") as writer:
        for y in range(half_height):
            row = reader.read(row_bytes)
            crop = bytearray(row[:crop_bytes])
            for index in range(half_width):
                at = index * bpp
                for channel in range(colour):
                    crop[at + channel] = 255 - crop[at + channel]
                if bpp > colour:
                    crop[at + colour] = 255
            writer.write(crop)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=REPO / "benchmarks/corpus_manifest.json")
    parser.add_argument("--cache", type=Path, default=REPO / "corpus-data/cache")
    parser.add_argument("--vpng", type=Path, default=REPO / "target/release/vpng.exe")
    parser.add_argument("--work", type=Path, default=REPO / "benchmarks/work/layers-memory")
    parser.add_argument("--output", type=Path, default=REPO / "benchmarks/p12_layers_memory_results.json")
    parser.add_argument("--workers", type=int, default=4)
    args = parser.parse_args()

    if not args.vpng.exists():
        print(f"no vpng binary at {args.vpng}; run `cargo build --release -p vpng-cli`")
        return 2

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    entry = next(
        (item for item in manifest["images"] if item["id"] == IMAGE_ID), None
    )
    if entry is None:
        print(f"the corpus does not hold {IMAGE_ID}")
        return 2

    raw = cache_index(args.cache).get(entry["sha256"])
    if raw is None:
        print(f"no cached canonical pixels for {IMAGE_ID}")
        return 2

    width, height, fmt = entry["width"], entry["height"], entry["pixel_format"]
    args.work.mkdir(parents=True, exist_ok=True)

    one = args.work / "one.vpng"
    two = args.work / "two.vpng"
    overlay = args.work / "overlay.raw"

    manifest_one = args.work / "one.json"
    manifest_one.write_text(
        json.dumps(
            {
                "layers": [
                    {
                        "name": "base",
                        "path": str(raw),
                        "width": width,
                        "height": height,
                        "format": fmt,
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    subprocess.run(
        [
            str(args.vpng), "encode-layers", str(one),
            "--manifest", str(manifest_one),
            "--workers", str(args.workers),
        ],
        check=True,
        capture_output=True,
    )

    write_overlay(raw, overlay, width, height, fmt)
    overlay_width = max(1, (width + 1) // 2)
    overlay_height = max(1, (height + 1) // 2)
    manifest_two = args.work / "two.json"
    manifest_two.write_text(
        json.dumps(
            {
                "layers": [
                    {
                        "name": "base",
                        "path": str(raw),
                        "width": width,
                        "height": height,
                        "format": fmt,
                    },
                    {
                        "name": "ink",
                        "path": str(overlay),
                        "x": 0,
                        "y": 0,
                        "width": overlay_width,
                        "height": overlay_height,
                        "format": fmt,
                    },
                ]
            }
        ),
        encoding="utf-8",
    )
    subprocess.run(
        [
            str(args.vpng), "encode-layers", str(two),
            "--manifest", str(manifest_two),
            "--workers", str(args.workers),
        ],
        check=True,
        capture_output=True,
    )

    cases = {
        "layers_json": [str(args.vpng), "layers", str(two), "--json"],
        "check": [str(args.vpng), "check", str(two)],
        "decode_layer_base": [
            str(args.vpng), "decode-layer", str(two),
            str(args.work / "base.raw"), "--layer", "0",
        ],
        "decode_layer_overlay": [
            str(args.vpng), "decode-layer", str(two),
            str(args.work / "overlay-decoded.raw"), "--layer", "1",
        ],
        "composite_one_layer": [
            str(args.vpng), "composite", str(one), str(args.work / "one.raw"),
        ],
        "composite_two_layers": [
            str(args.vpng), "composite", str(two), str(args.work / "two.raw"),
        ],
    }

    results = {}
    for name, command in cases.items():
        results[name] = measure(command)
        print(f"  {name}: {results[name]['peak_working_set_bytes']:,} bytes")

    payload = {
        "schema": "vpng.layers.memory.v1",
        "generated_by": "benchmarks/layers/measure_layer_memory.py",
        "image": {
            "id": IMAGE_ID,
            "width": width,
            "height": height,
            "pixel_format": fmt,
            "canonical_bytes": entry["canonical_bytes"],
        },
        "files": {
            "one_layer_bytes": one.stat().st_size,
            "two_layer_bytes": two.stat().st_size,
        },
        "scope": {
            "measured": "the process's peak working set, from the operating system's "
            "GetProcessMemoryInfo, taken while the process was alive",
            "included": "the operating system, the loader, the binary's text and data, "
            "the Rust heap, every buffer the command allocated, and the file it read",
            "excluded": "nothing: the figure is the whole process",
            "not_a_target": "this is a process figure for a whole-canvas operation; it is "
            "not the 64 MB localized-editing target, which is a different operation",
        },
        "cases": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=1), encoding="utf-8")
    print(f"wrote {args.output} ({args.output.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
