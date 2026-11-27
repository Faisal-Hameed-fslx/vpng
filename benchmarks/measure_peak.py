"""Measure a process's peak working set on Windows, exactly.

The operating system's `GetProcessMemoryInfo` reports the peak working set
of a process that has already finished, which is what the memory reports in
this repository need: the high-water mark, not a sample taken while the
process was alive.

Usage:

    python benchmarks/measure_peak.py <command> [arguments...]

It prints one JSON object:

    {"command": [...], "exit": 0, "peak_working_set_bytes": 12345}

**This module is Windows-only.** It calls `ctypes.windll.kernel32` and
`ctypes.windll.psapi`, which do not exist on other systems. The *import* is
portable — the platform calls are inside the function — so a script that only
needs the module's help text, or that never measures anything, still runs
everywhere. Only `peak_working_set` needs Windows.
"""

import ctypes
import json
import subprocess
import sys
from ctypes import wintypes


class ProcessMemoryCounters(ctypes.Structure):
    """The `PROCESS_MEMORY_COUNTERS` structure of `psapi.dll`."""

    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]


def peak_working_set(pid: int) -> int:
    """The peak working set of a process, in bytes."""
    counters = ProcessMemoryCounters()
    counters.cb = ctypes.sizeof(counters)
    handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)  # QUERY_INFORMATION
    if not handle:
        return 0
    try:
        ok = ctypes.windll.psapi.GetProcessMemoryInfo(
            handle, ctypes.byref(counters), counters.cb
        )
        return int(counters.PeakWorkingSetSize) if ok else 0
    finally:
        ctypes.windll.kernel32.CloseHandle(handle)


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    command = sys.argv[1:]
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    # The handle is taken while the process is alive, so the counters stay
    # readable after it exits.
    peak = 0
    while process.poll() is None:
        peak = max(peak, peak_working_set(process.pid))
    process.wait()
    peak = max(peak, peak_working_set(process.pid))
    payload = {
        "command": command,
        "exit": process.returncode,
        "peak_working_set_bytes": peak,
    }
    print(json.dumps(payload))
    return 0


if __name__ == "__main__":
    sys.exit(main())
