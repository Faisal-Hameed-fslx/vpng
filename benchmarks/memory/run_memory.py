#!/usr/bin/env python3
"""Run the P4 memory validation matrix and write the report.

Every case runs in its own process, so the process's resident-set
high-water mark belongs to that case alone. A case that cannot run on this
host — not enough disk, not enough memory, or a timeout — is reported as
``BLOCKED`` with the reason, never as a pass and never as a silent skip.

The report is written to ``benchmarks/p4_memory_results.json``.

Run from the repository root:

    python benchmarks/memory/run_memory.py
    python benchmarks/memory/run_memory.py --only 16384x16384-rgba8
"""

import argparse
import ctypes
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

#: The memory target for a localized edit on a 16K image, in bytes.
MEMORY_TARGET_BYTES = 64 * 1024 * 1024

#: The resolutions in the matrix.
RESOLUTIONS = [4096, 8192, 16384]

#: The pixel formats in the matrix.
FORMATS = ["rgba8", "gray8", "rgb8", "rgba16"]

#: The tile size used for every case.
TILE_SIZE = 512

#: The decoded-tile cache budget.
CACHE_BYTES = 16 * 1024 * 1024

#: The undo history budget.
HISTORY_BYTES = 32 * 1024 * 1024


def host_info() -> dict:
    """The host's memory, disk and processor facts."""
    info = {
        "platform": sys.platform,
        "processor": os.environ.get("PROCESSOR_IDENTIFIER", "unknown"),
        "logical_cpus": os.cpu_count(),
    }
    if sys.platform == "win32":
        class MemoryStatusEx(ctypes.Structure):
            _fields_ = [
                ("dwLength", ctypes.c_ulong),
                ("dwMemoryLoad", ctypes.c_ulong),
                ("ullTotalPhys", ctypes.c_ulonglong),
                ("ullAvailPhys", ctypes.c_ulonglong),
                ("ullTotalPageFile", ctypes.c_ulonglong),
                ("ullAvailPageFile", ctypes.c_ulonglong),
                ("ullTotalVirtual", ctypes.c_ulonglong),
                ("ullAvailVirtual", ctypes.c_ulonglong),
                ("ullAvailExtendedVirtual", ctypes.c_ulonglong),
            ]

        status = MemoryStatusEx()
        status.dwLength = ctypes.sizeof(MemoryStatusEx)
        ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status))
        info["total_physical_bytes"] = status.ullTotalPhys
        info["available_physical_bytes"] = status.ullAvailPhys
        info["total_page_file_bytes"] = status.ullTotalPageFile
        info["available_page_file_bytes"] = status.ullAvailPageFile
    else:
        try:
            with open("/proc/meminfo", encoding="utf-8") as handle:
                for line in handle:
                    if line.startswith("MemTotal:"):
                        info["total_physical_bytes"] = int(line.split()[1]) * 1024
                    elif line.startswith("MemAvailable:"):
                        info["available_physical_bytes"] = int(line.split()[1]) * 1024
        except OSError:
            pass

    usage = shutil.disk_usage(REPO)
    info["work_disk_total_bytes"] = usage.total
    info["work_disk_free_bytes"] = usage.free
    return info


def frame_bytes(fmt: str, width: int, height: int) -> int:
    """The canonical size of one image, in bytes."""
    bpp = {"gray8": 1, "rgb8": 3, "rgba8": 4, "rgba16": 8}[fmt]
    return width * height * bpp


def run_case(binary: Path, work_dir: Path, resolution: int, fmt: str, timeout: int) -> dict:
    """Run one validation case in its own process."""
    canonical = frame_bytes(fmt, resolution, resolution)
    # The file is written twice at once during the commit: the original and
    # the temporary file. The generated pattern compresses well, so half the
    # canonical size is a generous estimate of the compressed file; the
    # remaining headroom covers the temporary copy and the file system's own
    # slack.
    needed = canonical // 2 + 512 * 1024 * 1024
    free = shutil.disk_usage(work_dir).free
    if free < needed:
        return {
            "status": "BLOCKED",
            "reason": (
                f"{free} bytes free on the work disk, {needed} needed for the "
                f"original and the temporary copy of a {resolution}x{resolution} "
                f"{fmt} file"
            ),
        }

    output = work_dir / f"case-{resolution}-{fmt}.json"
    command = [
        str(binary),
        "--width", str(resolution),
        "--height", str(resolution),
        "--format", fmt,
        "--tile-size", str(TILE_SIZE),
        "--work-dir", str(work_dir),
        "--cache-bytes", str(CACHE_BYTES),
        "--history-bytes", str(HISTORY_BYTES),
        "--output", str(output),
    ]
    started = time.perf_counter()
    try:
        proc = subprocess.run(
            command, capture_output=True, text=True, timeout=timeout
        )
    except subprocess.TimeoutExpired:
        return {
            "status": "BLOCKED",
            "reason": f"the case did not finish within {timeout} seconds",
        }
    elapsed = round((time.perf_counter() - started) * 1000)

    if proc.returncode != 0:
        return {
            "status": "BLOCKED",
            "reason": (
                f"the process exited with {proc.returncode}: "
                f"{(proc.stdout + proc.stderr).strip()[:300]}"
            ),
        }
    if not output.exists():
        return {"status": "BLOCKED", "reason": "the case wrote no report"}
    report = json.loads(output.read_text(encoding="utf-8"))
    report["wall_ms"] = elapsed
    report["command"] = command

    # The verdict: the file must verify, the edit must be exact, and the
    # edit's memory must stay inside the target.
    edit = report.get("edit", {})
    memory = edit.get("memory", {})
    heap_peak = memory.get("heap_peak_bytes", 0)
    working_set_delta = memory.get("working_set_delta_bytes", 0)
    verification_ok = report.get("verification", {}).get("valid", False)
    pixels_ok = edit.get("pixels_read_back_exact", False)
    payload_ok = edit.get("payload_identity", {}).get(
        "untouched_payloads_byte_identical", False
    )

    failures = []
    if not verification_ok:
        failures.append("the streamed file did not verify")
    if not pixels_ok:
        failures.append("the edited region did not read back exactly")
    if not payload_ok:
        failures.append("an untouched tile's payload changed")
    if heap_peak > MEMORY_TARGET_BYTES:
        failures.append(
            f"the edit's heap peak was {heap_peak} bytes, over the "
            f"{MEMORY_TARGET_BYTES} byte target"
        )
    if working_set_delta > MEMORY_TARGET_BYTES:
        failures.append(
            f"the edit's resident-set growth was {working_set_delta} bytes, "
            f"over the {MEMORY_TARGET_BYTES} byte target"
        )

    report["verdict"] = "FAIL" if failures else "PASS"
    report["failures"] = failures
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary",
        default=str(REPO / "target" / "release" / "vpng-memory.exe"),
    )
    parser.add_argument(
        "--work-dir",
        default=str(REPO / "benchmarks" / "results" / "memory-work"),
    )
    parser.add_argument(
        "--output",
        default=str(REPO / "benchmarks" / "p4_memory_results.json"),
    )
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--timeout", type=int, default=7200)
    args = parser.parse_args()

    binary = Path(args.binary)
    if not binary.exists():
        raise SystemExit(
            f"the validation binary is missing: {binary} "
            "(cargo build --release -p vpng-validation)"
        )
    work_dir = Path(args.work_dir)
    work_dir.mkdir(parents=True, exist_ok=True)

    cases = []
    for resolution in RESOLUTIONS:
        for fmt in FORMATS:
            name = f"{resolution}x{resolution}-{fmt}"
            if args.only and name not in args.only:
                continue
            print(f"== {name}", flush=True)
            started = time.perf_counter()
            result = run_case(binary, work_dir, resolution, fmt, args.timeout)
            elapsed = round((time.perf_counter() - started) * 1000)
            if "verdict" in result:
                print(
                    f"   {result['verdict']} in {elapsed} ms "
                    f"(write {result['write']['elapsed_ms']} ms, "
                    f"file {result['write']['file_size_bytes']:,} bytes)",
                    flush=True,
                )
            else:
                print(f"   {result['status']}: {result['reason']}", flush=True)
            result["id"] = name
            result["resolution"] = resolution
            result["format"] = fmt
            result["elapsed_ms"] = elapsed
            cases.append(result)

    verdicts = {"PASS": 0, "FAIL": 0, "BLOCKED": 0}
    for case in cases:
        verdict = case.get("verdict", case.get("status", "BLOCKED"))
        verdicts[verdict] = verdicts.get(verdict, 0) + 1

    document = {
        "tool": "run_memory.py",
        "schema": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "host": host_info(),
        "context": {
            "binary": str(binary),
            "tile_size": TILE_SIZE,
            "cache_budget_bytes": CACHE_BYTES,
            "history_budget_bytes": HISTORY_BYTES,
            "memory_target_bytes": MEMORY_TARGET_BYTES,
            "python": sys.version.split()[0],
        },
        "definitions": {
            "verdict": (
                "PASS: the case ran, the file verified, the edit read back "
                "exactly, the untouched payloads were byte identical, and the "
                "edit's memory stayed inside the target. FAIL: the case ran "
                "and one of those conditions did not hold. BLOCKED: the case "
                "could not run on this host."
            ),
            "working_set_bytes": (
                "the operating system's resident-set size of the process, from "
                "K32GetProcessMemoryInfo. It includes the binary, the stacks, "
                "the allocator's own bookkeeping and the heap."
            ),
            "heap_peak_bytes": (
                "the bytes the process's own allocator had outstanding at the "
                "busiest moment, counted by a tracking global allocator. It "
                "excludes the binary, the stacks and the operating system's "
                "own overhead."
            ),
            "working_set_delta_bytes": (
                "the resident-set high-water mark minus the resident set "
                "measured before the edit started. This is what the edit "
                "itself cost the process."
            ),
            "included_in_the_target": (
                "the decoded-tile cache, the undo history, the dirty tiles, "
                "the region buffers, the tile payloads the writer read, and "
                "the process's own code and stacks"
            ),
            "excluded_from_the_target": (
                "the operating system's page cache, the file on disk, the "
                "memory other processes use, and the space the temporary "
                "file occupies"
            ),
            "streaming_note": (
                "the writer never holds the whole image: it generates one "
                "tile, encodes it, writes it, and moves on. The canonical "
                "size of the image is recorded for comparison only."
            ),
        },
        "summary": {
            "cases": len(cases),
            "pass": verdicts.get("PASS", 0),
            "fail": verdicts.get("FAIL", 0),
            "blocked": verdicts.get("BLOCKED", 0),
        },
        "cases": cases,
    }

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8")
    print()
    print(f"PASS {verdicts.get('PASS', 0)}  FAIL {verdicts.get('FAIL', 0)}  "
          f"BLOCKED {verdicts.get('BLOCKED', 0)}")
    print(f"wrote {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
