#!/usr/bin/env python3
"""Measure the Python extension against the `vpng` command line.

The question this answers is narrow and answerable: **does the Python API
write the same bytes as the command line?** If it does, the binding is a
faithful wrapper; if it does not, one of them is lying about the format.

For every image in the sample the script:

* encodes it with the real `vpng` binary, from the same canonical pixels;
* encodes it with the Python extension, from the same canonical pixels;
* compares the two files **byte for byte** by SHA-256;
* runs `vpng check` and `vpng verify` on the **Python-written** file, so a
  smaller-but-wrong file fails the run instead of being reported as a win;
* encodes it again through Python with a different worker count and compares
  the digest, so the worker count is shown not to move the bytes;
* records the wall clock of both paths and the peak working set of the
  Python process.

Run from the repository root:

    python benchmarks/python/run_python_bench.py --sample 100

The raw results are written to `benchmarks/p9_python_results.json`, and
`check_python()` in `benchmarks/verify_reports.py` binds the figures the
report quotes to that file.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import math
import os
import platform
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "python"))

CHUNK = 1024 * 1024


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
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(CHUNK)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def cache_index(cache_root: Path) -> dict[str, Path]:
    """Map every cached canonical digest to its raw file.

    The cache is keyed by the digest of the *source* file, and the sidecar
    records the digest of the canonical pixels as well. The corpus manifest
    is keyed by the canonical digest, so the sidecars are the join.
    """
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


def peak_working_set_bytes() -> int | None:
    """The peak working set of this process, or `None` where it is unknown.

    On Windows this reads `PROCESS_MEMORY_COUNTERS.PeakWorkingSetSize` for
    the current process. Everywhere else it returns `None`, and the report
    says the figure was not measured rather than guessing.

    **Included:** the whole process — the interpreter, the extension module,
    every pixel buffer the encode holds, the tile payloads in flight, the
    encoded payloads waiting to be written, the container metadata, the
    stacks and the allocator's own bookkeeping.
    **Excluded:** the canonical pixel file on disk, the operating system's
    page cache, and the memory of other processes.
    """
    if platform.system() != "Windows":
        return None

    class ProcessMemoryCounters(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.c_ulong),
            ("PageFaultCount", ctypes.c_ulong),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    counters = ProcessMemoryCounters()
    counters.cb = ctypes.sizeof(counters)

    kernel32 = ctypes.windll.kernel32
    psapi = ctypes.windll.psapi
    # The signatures matter: without them ctypes passes a 32-bit `int` where
    # a 64-bit handle is expected, and the call fails.
    kernel32.GetCurrentProcess.restype = ctypes.c_void_p
    psapi.GetProcessMemoryInfo.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ProcessMemoryCounters),
        ctypes.c_ulong,
    ]
    psapi.GetProcessMemoryInfo.restype = ctypes.c_int

    handle = kernel32.GetCurrentProcess()
    if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
        return None
    return int(counters.PeakWorkingSetSize)


def run_vpng(vpng: Path, args: list[str], timeout: int = 3600) -> dict:
    """Run a `vpng` subcommand and time it."""
    started = time.perf_counter()
    proc = subprocess.run(
        [str(vpng), *args], capture_output=True, text=True, timeout=timeout
    )
    elapsed = (time.perf_counter() - started) * 1000.0
    return {
        "exit": proc.returncode,
        "ms": round(elapsed, 3),
        "output": proc.stdout.strip(),
        "error": proc.stderr.strip(),
    }


def read_header(path: Path) -> dict:
    """The header fields the report quotes."""
    body = path.read_bytes()
    import struct

    major, minor = struct.unpack_from("<HH", body, 8)
    width, height = struct.unpack_from("<II", body, 16)
    return {
        "major": major,
        "minor": minor,
        "width": width,
        "height": height,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--sample",
        type=int,
        default=100,
        help="how many corpus images to measure (the whole corpus is 750)",
    )
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--alt-workers", type=int, default=1)
    parser.add_argument(
        "--vpng", default=str(REPO / "target" / "release" / "vpng.exe")
    )
    parser.add_argument("--cache-root", default=str(REPO / "corpus-data" / "cache"))
    parser.add_argument(
        "--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json")
    )
    parser.add_argument(
        "--output", default=str(REPO / "benchmarks" / "p9_python_results.json")
    )
    parser.add_argument("--work", default=str(REPO / "benchmarks" / "work" / "python"))
    args = parser.parse_args()

    vpng = Path(args.vpng)
    if not vpng.exists():
        print(f"vpng binary not found: {vpng} (cargo build --release --workspace)")
        return 2

    try:
        import vpng as vpng_py
    except ImportError as error:
        print(f"the vpng Python package is not importable: {error}")
        print("build it with: maturin develop --release")
        return 2

    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    cache = cache_index(Path(args.cache_root))

    # The sample is deterministic: the manifest's own order, which the corpus
    # builder sorted by the digest of the path.
    candidates = []
    for entry in manifest.get("images", []):
        raw = cache.get(entry["sha256"])
        if raw is not None:
            candidates.append((entry, raw))

    sample = candidates[: args.sample]
    print(f"corpus images with cached canonical pixels: {len(candidates)}")
    print(f"sample: {len(sample)}")

    work = Path(args.work)
    work.mkdir(parents=True, exist_ok=True)

    records = []
    for number, (entry, raw) in enumerate(sample, start=1):
        width, height = entry["width"], entry["height"]
        fmt = entry["pixel_format"]
        pixels = raw.read_bytes()

        cli_out = work / "cli.vpng"
        py_out = work / "python.vpng"
        alt_out = work / "python-alt.vpng"

        base = [
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

        cli = run_vpng(vpng, ["encode", str(raw), str(cli_out), *base])

        started = time.perf_counter()
        report = vpng_py.encode_pixels(
            pixels,
            width,
            height,
            py_out,
            format=fmt,
            tile_size=args.tile_size,
            workers=args.workers,
        )
        python_wall_ms = (time.perf_counter() - started) * 1000.0

        alt = vpng_py.encode_pixels(
            pixels,
            width,
            height,
            alt_out,
            format=fmt,
            tile_size=args.tile_size,
            workers=args.alt_workers,
        )

        cli_digest = file_sha256(cli_out)
        py_digest = file_sha256(py_out)
        alt_digest = file_sha256(alt_out)

        check = run_vpng(vpng, ["check", str(py_out)])
        verify = run_vpng(vpng, ["verify", str(raw), str(py_out), *base])

        records.append(
            {
                "id": entry["id"],
                "category": entry["category"],
                "width": width,
                "height": height,
                "pixel_format": fmt,
                "canonical_bytes": entry["canonical_bytes"],
                "file_bytes": py_out.stat().st_size,
                "cli_ms": cli["ms"],
                "python_wall_ms": round(python_wall_ms, 3),
                "python_encode_ms": report.encode_ms,
                "cli_sha256": cli_digest,
                "python_sha256": py_digest,
                "identical": cli_digest == py_digest,
                "alt_workers": args.alt_workers,
                "alt_workers_sha256": alt_digest,
                "deterministic": alt_digest == py_digest,
                "header": read_header(py_out),
                "check_exit": check["exit"],
                "verify_exit": verify["exit"],
            }
        )

        if number % 10 == 0 or number == len(sample):
            print(f"  {number}/{len(sample)} measured")

        for path in (cli_out, py_out, alt_out):
            path.unlink(missing_ok=True)

    identical = sum(1 for record in records if record["identical"])
    deterministic = sum(1 for record in records if record["deterministic"])
    check_failures = sum(1 for record in records if record["check_exit"] != 0)
    verify_failures = sum(1 for record in records if record["verify_exit"] != 0)

    summary = {
        "sample": len(records),
        "identical": identical,
        "identical_differing": len(records) - identical,
        "deterministic": deterministic,
        "deterministic_differing": len(records) - deterministic,
        "check_failures": check_failures,
        "verify_failures": verify_failures,
        "file_bytes_total": sum(record["file_bytes"] for record in records),
        "canonical_bytes_total": sum(
            record["canonical_bytes"] for record in records
        ),
        "cli_ms": distribution([record["cli_ms"] for record in records]),
        "python_wall_ms": distribution(
            [record["python_wall_ms"] for record in records]
        ),
        "python_encode_ms": distribution(
            [record["python_encode_ms"] for record in records]
        ),
        "peak_working_set_bytes": peak_working_set_bytes(),
    }

    payload = {
        "generated": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
            "machine": platform.machine(),
        },
        "settings": {
            "tile_size": args.tile_size,
            "workers": args.workers,
            "alt_workers": args.alt_workers,
            "vpng_binary": str(vpng),
            "sample_rule": "the manifest order, which the corpus builder sorted by the digest of the path",
        },
        "summary": summary,
        "images": records,
    }
    Path(args.output).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    print()
    print(f"sample:                    {summary['sample']}")
    print(f"byte-identical to the CLI: {identical}/{summary['sample']}")
    print(f"worker count changed bytes: {summary['deterministic_differing']}")
    print(f"check failures:            {summary['check_failures']}")
    print(f"verify failures:           {summary['verify_failures']}")
    print(f"CLI wall median:           {summary['cli_ms']['median']} ms")
    print(f"Python wall median:        {summary['python_wall_ms']['median']} ms")
    print(f"Python encode median:      {summary['python_encode_ms']['median']} ms")
    print(f"peak working set:          {summary['peak_working_set_bytes']}")
    print(f"written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
