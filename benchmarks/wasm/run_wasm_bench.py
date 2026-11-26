#!/usr/bin/env python3
"""Measure the WebAssembly package over the whole 750-image corpus.

The question this answers is narrow and answerable: **does the browser's
copy of the engine produce the command line's pixels, and what does it cost?**

For every image in the corpus the script:

* encodes it locally with the real `vpng` binary, from the same canonical
  pixels the other corpus reports used;
* decodes a 1024x1024 viewport with the native reader, through the same
  `RangeReader` the WebAssembly module uses, and records the timings and the
  canonical digest;
* hands the same file to the WebAssembly module and measures two paths:
  the **whole-file** path, where the container is copied into the module's
  memory, and the **range** path, where the module is opened with the header
  and the directory alone and only the tiles the viewport touches are
  fetched;
* compares all three canonical digests, so a module that produced different
  pixels would fail the run instead of being reported as a win.

Run from the repository root:

    python benchmarks/wasm/run_wasm_bench.py --sample 750

The raw results are written to `benchmarks/p11_wasm_results.json`, and
`check_wasm()` in `benchmarks/verify_reports.py` binds the figures the
report quotes to that file.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

CHUNK = 1024 * 1024


# --------------------------------------------------------------------------
# The measurement rules. They are the same rules the other corpus reports
# use, so the numbers can be read side by side.
# --------------------------------------------------------------------------


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
    """The nearest-rank p95: the smallest sample at or above 95% of the rank."""
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


def file_sha256(path: Path) -> str:
    digest = __import__("hashlib").sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(CHUNK)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


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
        if not digest or not name:
            continue
        raw = cache_root / name
        if raw.exists():
            index[digest] = raw
    return index


def run_tool(command: list[str], timeout: int = 3600) -> subprocess.CompletedProcess:
    """Run a tool and return the completed process."""
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--sample",
        type=int,
        default=750,
        help="how many corpus images to measure (the whole corpus is 750)",
    )
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument(
        "--viewport", type=int, default=1024, help="the viewport edge in pixels"
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=5,
        help="how many times the native path repeats each region decode",
    )
    parser.add_argument(
        "--vpng", default=str(REPO / "target" / "release" / "vpng.exe")
    )
    parser.add_argument(
        "--bench",
        default=str(REPO / "target" / "release" / "vpng-region-bench.exe"),
    )
    parser.add_argument(
        "--wasm",
        default=str(
            REPO / "target" / "wasm32-unknown-unknown" / "release" / "vpng_wasm.wasm"
        ),
    )
    parser.add_argument("--node", default="node")
    parser.add_argument("--cache-root", default=str(REPO / "corpus-data" / "cache"))
    parser.add_argument(
        "--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json")
    )
    parser.add_argument(
        "--output", default=str(REPO / "benchmarks" / "p11_wasm_results.json")
    )
    parser.add_argument("--work", default=str(REPO / "benchmarks" / "work" / "wasm"))
    args = parser.parse_args()

    vpng = Path(args.vpng)
    bench = Path(args.bench)
    wasm = Path(args.wasm)
    for tool, what in ((vpng, "vpng"), (bench, "vpng-region-bench"), (wasm, "the wasm module")):
        if not tool.exists():
            print(f"{what} not found: {tool}")
            return 2

    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    cache = cache_index(Path(args.cache_root))

    candidates = []
    for entry in manifest.get("images", []):
        raw = cache.get(entry["sha256"])
        if raw is not None:
            candidates.append((entry, raw))

    sample = candidates[: args.sample]
    print(f"corpus images with cached canonical pixels: {len(candidates)}")
    print(f"sample: {len(sample)}")

    work = Path(args.work)
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True, exist_ok=True)

    jobs = []
    native = {}
    for number, (entry, raw) in enumerate(sample, start=1):
        width, height = entry["width"], entry["height"]
        fmt = entry["pixel_format"]
        name = f"{entry['sha256'][:16]}.vpng"
        local = work / name

        encode = run_tool(
            [
                str(vpng),
                "encode",
                str(raw),
                str(local),
                "--input-format",
                "raw",
                "--width",
                str(width),
                "--height",
                str(height),
                "--format",
                fmt,
                "--tile-size",
                str(args.tile_size),
                "--workers",
                str(args.workers),
            ]
        )
        if encode.returncode != 0:
            raise RuntimeError(f"encode failed for {entry['id']}: {encode.stderr}")

        view_w = min(width, args.viewport)
        view_h = min(height, args.viewport)

        # The native path, through the same reader the module uses.
        native_run = run_tool(
            [
                str(bench),
                str(local),
                "--x",
                "0",
                "--y",
                "0",
                "--width",
                str(view_w),
                "--height",
                str(view_h),
                "--runs",
                str(args.runs),
                "--source",
                "memory",
            ]
        )
        if native_run.returncode != 0:
            raise RuntimeError(
                f"the native region decode failed for {entry['id']}: {native_run.stderr}"
            )
        native_report = json.loads(native_run.stdout)

        native_run_file = run_tool(
            [
                str(bench),
                str(local),
                "--x",
                "0",
                "--y",
                "0",
                "--width",
                str(view_w),
                "--height",
                str(view_h),
                "--runs",
                str(args.runs),
                "--source",
                "file",
            ]
        )
        if native_run_file.returncode != 0:
            raise RuntimeError(
                f"the native streaming decode failed for {entry['id']}: "
                f"{native_run_file.stderr}"
            )
        native_file_report = json.loads(native_run_file.stdout)

        native[entry["id"]] = {
            "memory_median_ms": native_report["median_ms"],
            "memory_samples_ms": native_report["samples_ms"],
            "file_median_ms": native_file_report["median_ms"],
            "file_samples_ms": native_file_report["samples_ms"],
            "bytes_read": native_file_report["bytes_read"],
            "payloads_read": native_file_report["payloads_read"],
            "sha256": native_report["sha256"],
        }
        jobs.append(
            {
                "id": entry["id"],
                "file": name,
                "x": 0,
                "y": 0,
                "width": view_w,
                "height": view_h,
            }
        )

        if number % 25 == 0 or number == len(sample):
            print(f"  {number}/{len(sample)} encoded and measured natively")

    # The module's half of the measurement.
    jobs_path = work / "jobs.json"
    jobs_path.write_text(json.dumps(jobs, indent=2), encoding="utf-8")
    wasm_results_path = work / "wasm-results.json"
    node = run_tool(
        [
            args.node,
            str(REPO / "wasm" / "run_wasm_bench.mjs"),
            "--wasm",
            str(wasm),
            "--dir",
            str(work),
            "--jobs",
            str(jobs_path),
            "--output",
            str(wasm_results_path),
        ],
        timeout=7200,
    )
    if node.returncode != 0:
        raise RuntimeError(f"the wasm measurement failed: {node.stderr}")
    wasm_results = json.loads(wasm_results_path.read_text(encoding="utf-8"))
    wasm_by_id = {record["id"]: record for record in wasm_results}

    records = []
    for entry, _raw in sample:
        image_id = entry["id"]
        wasm_record = wasm_by_id.get(image_id)
        native_record = native[image_id]
        if wasm_record is None:
            raise RuntimeError(f"the module did not measure {image_id}")

        whole_identical = wasm_record["whole_sha256"] == native_record["sha256"]
        range_identical = wasm_record["range_sha256"] == native_record["sha256"]
        records.append(
            {
                "id": image_id,
                "category": entry["category"],
                "width": entry["width"],
                "height": entry["height"],
                "pixel_format": entry["pixel_format"],
                "canonical_bytes": entry["canonical_bytes"],
                "file_bytes": wasm_record["file_bytes"],
                "header_bytes": wasm_record["header_bytes"],
                "directory_bytes": wasm_record["directory_bytes"],
                "viewport_width": wasm_record["viewport_width"],
                "viewport_height": wasm_record["viewport_height"],
                "format_code": wasm_record["format"],
                "tiles": wasm_record["tiles"],
                "levels": wasm_record["levels"],
                "native_memory_median_ms": native_record["memory_median_ms"],
                "native_memory_samples_ms": native_record["memory_samples_ms"],
                "native_file_median_ms": native_record["file_median_ms"],
                "native_file_samples_ms": native_record["file_samples_ms"],
                "native_bytes_read": native_record["bytes_read"],
                "native_payloads_read": native_record["payloads_read"],
                "wasm_whole_ms": wasm_record["whole_ms"],
                "wasm_range_ms": wasm_record["range_ms"],
                "wasm_range_bytes": wasm_record["range_bytes"],
                "wasm_range_payload_bytes": wasm_record["range_payload_bytes"],
                "wasm_range_requests": wasm_record["range_requests"],
                "native_sha256": native_record["sha256"],
                "wasm_whole_sha256": wasm_record["whole_sha256"],
                "wasm_range_sha256": wasm_record["range_sha256"],
                "whole_identical": whole_identical,
                "range_identical": range_identical,
                "range_fraction": round(
                    wasm_record["range_bytes"] / wasm_record["file_bytes"], 6
                ),
            }
        )

    identical = sum(1 for record in records if record["whole_identical"])
    range_identical = sum(1 for record in records if record["range_identical"])
    total_object = sum(record["file_bytes"] for record in records)
    total_range = sum(record["wasm_range_bytes"] for record in records)
    total_native = sum(record["native_bytes_read"] for record in records)

    by_category: dict[str, dict] = {}
    for record in records:
        bucket = by_category.setdefault(
            record["category"],
            {
                "images": 0,
                "object_bytes": 0,
                "range_bytes": 0,
                "native_bytes_read": 0,
                "identical": 0,
                "range_identical": 0,
            },
        )
        bucket["images"] += 1
        bucket["object_bytes"] += record["file_bytes"]
        bucket["range_bytes"] += record["wasm_range_bytes"]
        bucket["native_bytes_read"] += record["native_bytes_read"]
        bucket["identical"] += 1 if record["whole_identical"] else 0
        bucket["range_identical"] += 1 if record["range_identical"] else 0
    for bucket in by_category.values():
        bucket["range_fraction"] = round(
            bucket["range_bytes"] / bucket["object_bytes"], 6
        )

    summary = {
        "sample": len(records),
        "identical": identical,
        "identical_differing": len(records) - identical,
        "range_identical": range_identical,
        "range_identical_differing": len(records) - range_identical,
        "object_bytes_total": total_object,
        "range_bytes_total": total_range,
        "native_bytes_read_total": total_native,
        "range_fraction": round(total_range / total_object, 6) if total_object else None,
        "native_fraction": round(total_native / total_object, 6)
        if total_object
        else None,
        "wasm_whole_ms": distribution([record["wasm_whole_ms"] for record in records]),
        "wasm_range_ms": distribution([record["wasm_range_ms"] for record in records]),
        "native_memory_ms": distribution(
            [record["native_memory_median_ms"] for record in records]
        ),
        "native_file_ms": distribution(
            [record["native_file_median_ms"] for record in records]
        ),
        "by_category": by_category,
    }

    payload = {
        "generated": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
            "machine": platform.machine(),
            "node": subprocess.run(
                [args.node, "--version"], capture_output=True, text=True
            ).stdout.strip(),
        },
        "settings": {
            "tile_size": args.tile_size,
            "workers": args.workers,
            "viewport": args.viewport,
            "native_runs": args.runs,
            "vpng_binary": str(vpng),
            "wasm_module": str(wasm),
            "sample_rule": "the manifest order, which the corpus builder sorted by the digest of the path",
            "memory_scope": {
                "included": "nothing: this measurement reports bytes moved and decode times, not process memory",
                "excluded": "the process working set of the driver, of the Node process and of the native bench",
            },
            "timing_scope": {
                "native_memory": "the region decode through RangeReader over bytes already in memory, timed in-process; the container is read from disk outside the measurement",
                "native_file": "the region decode through RangeReader over the file on disk, timed in-process; the disk read is inside the measurement",
                "wasm_whole": "the copy of the container into the module's memory plus the region decode, timed in-process; the file read from disk is outside the measurement",
                "wasm_range": "the open from the header and the directory plus the fetch of the tiles the viewport touches plus the decode, timed in-process; the bytes are fetched from an in-process slice, not from a network",
            },
        },
        "summary": summary,
        "images": records,
    }
    Path(args.output).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    print()
    print(f"sample:                       {summary['sample']}")
    print(f"identical to the native read: {identical}/{summary['sample']}")
    print(f"identical on the range path:  {range_identical}/{summary['sample']}")
    print(f"object bytes:                 {summary['object_bytes_total']}")
    print(f"bytes for a range viewport:   {summary['range_bytes_total']}")
    print(f"fraction for a range viewport:{summary['range_fraction']}")
    print(f"native bytes read:            {summary['native_bytes_read_total']}")
    print(f"native memory median:         {summary['native_memory_ms']['median']} ms")
    print(f"native file median:           {summary['native_file_ms']['median']} ms")
    print(f"wasm whole median:            {summary['wasm_whole_ms']['median']} ms")
    print(f"wasm range median:            {summary['wasm_range_ms']['median']} ms")
    print(f"written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
