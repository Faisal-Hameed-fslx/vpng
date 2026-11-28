#!/usr/bin/env python3
"""Measure what the GPU payload costs in memory, on the 16K photograph.

A GPU payload is a **second copy of the picture** in a block format, so it
costs bytes in the file and bytes in memory. This script measures that cost,
exactly, so the milestone report can state it instead of leaving it to
inference.

It measures the **peak working set** of each command, taken from the
operating system's own `GetProcessMemoryInfo` while the process is alive.
The figure is the process's high-water mark: the operating system, the
loader, the binary's own text and data, the Rust heap and every buffer the
command allocated are all inside it.

Run from the repository root:

    python benchmarks/gpu/measure_gpu_memory.py
"""

import argparse
import json
import subprocess
import sys
import tempfile
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
    """Run a command and report its peak working set and its time.

    The child's output goes to **temporary files**, not to pipes. A pipe
    holds only a few kilobytes before the writer blocks, and this driver
    never reads the pipes while the child runs — so a command that prints
    more than a pipe's worth (a `gpu-info --json` over two thousand tiles,
    for one) would wait for a reader that never comes. A file never
    blocks, and the output is still read back afterwards.
    """
    started = time.perf_counter()
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        process = subprocess.Popen(command, stdout=out, stderr=err)
        peak = 0
        while process.poll() is None:
            peak = max(peak, peak_working_set(process.pid))
        process.wait()
        peak = max(peak, peak_working_set(process.pid))
        out.seek(0)
        err.seek(0)
        stdout = out.read().decode("utf-8", "replace")
        stderr = err.read().decode("utf-8", "replace")
    return {
        "command": command[1:],
        "exit": process.returncode,
        "peak_working_set_bytes": peak,
        "elapsed_ms": round((time.perf_counter() - started) * 1000),
        "stdout_bytes": len(stdout),
        "stderr_tail": stderr[-500:],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=REPO / "benchmarks/corpus_manifest.json")
    parser.add_argument("--cache", type=Path, default=REPO / "corpus-data/cache")
    parser.add_argument("--vpng", type=Path, default=REPO / "target/release/vpng.exe")
    parser.add_argument("--work", type=Path, default=REPO / "benchmarks/work/gpu-memory")
    parser.add_argument("--output", type=Path, default=REPO / "benchmarks/p13_gpu_memory_results.json")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--tile-size", type=int, default=256)
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

    plain = args.work / "plain.vpng"
    gpu_file = args.work / "gpu.vpng"

    common = [
        "--input-format", "raw",
        "--width", str(width),
        "--height", str(height),
        "--format", fmt,
        "--tile-size", str(args.tile_size),
        "--workers", str(args.workers),
    ]

    # 1. The plain encode: the baseline the GPU encode's cost is measured
    # against.
    plain_case = measure(
        [str(args.vpng), "encode", str(raw), str(plain), *common]
    )
    print(f"  encode: {plain_case['peak_working_set_bytes']:,} bytes")

    # 2. The GPU encode: the same pixels, plus a GPU payload beside every
    # tile's lossless payload.
    gpu_case = measure(
        [str(args.vpng), "encode", str(raw), str(gpu_file), *common, "--gpu"]
    )
    print(f"  encode --gpu: {gpu_case['peak_working_set_bytes']:,} bytes")

    # 3. The commands that read the file.
    cases = {
        "encode_plain": plain_case,
        "encode_gpu": gpu_case,
        "gpu_info": measure(
            [str(args.vpng), "gpu-info", str(gpu_file), "--json"]
        ),
        "check": measure([str(args.vpng), "check", str(gpu_file)]),
        "decode_exact": measure(
            [
                str(args.vpng), "decode", str(gpu_file),
                str(args.work / "exact.raw"), "--format", "raw",
            ]
        ),
        "decode_gpu": measure(
            [
                str(args.vpng), "gpu-decode", str(gpu_file),
                str(args.work / "gpu.raw"), "--format", "raw",
                "--x", "0", "--y", "0",
                "--width", str(width), "--height", str(height),
            ]
        ),
    }
    for name, case in cases.items():
        print(f"  {name}: {case['peak_working_set_bytes']:,} bytes")

    # The table's own accounting, read out of the file rather than inferred:
    # the payloads' bytes and the table's bytes are separate, and the file's
    # growth is the two of them plus the eight bytes the header grew by.
    table_payload_bytes = 0
    table_bytes = 0
    info = subprocess.run(
        [str(args.vpng), "gpu-info", str(gpu_file), "--json"],
        capture_output=True,
        text=True,
        check=True,
    )
    for key, target in (("gpu_payload_bytes", "payload"), ("gpu_table_length", "table")):
        marker = f'"{key}":'
        for part in info.stdout.split(marker)[1:]:
            value = int(part.split(",")[0])
            if target == "payload":
                table_payload_bytes = value
            else:
                table_bytes = value
            break

    print(
        f"  file accounting: payloads {table_payload_bytes:,} B, "
        f"table {table_bytes:,} B"
    )

    payload = {
        "schema": "vpng.gpu.memory.v1",
        "generated_by": "benchmarks/gpu/measure_gpu_memory.py",
        "image": {
            "id": IMAGE_ID,
            "width": width,
            "height": height,
            "pixel_format": fmt,
            "canonical_bytes": entry["canonical_bytes"],
            "tile_size": args.tile_size,
        },
        "files": {
            "plain_bytes": plain.stat().st_size,
            "gpu_bytes": gpu_file.stat().st_size,
            "gpu_overhead_bytes": gpu_file.stat().st_size - plain.stat().st_size,
            "gpu_payload_bytes": table_payload_bytes,
            "gpu_table_bytes": table_bytes,
        },
        "scope": {
            "measured": "the process's peak working set, from the operating system's "
            "GetProcessMemoryInfo, taken while the process was alive",
            "included": "the operating system, the loader, the binary's text and data, "
            "the Rust heap, every buffer the command allocated, and the file it read",
            "excluded": "nothing: the figure is the whole process",
            "not_a_target": "this is a process figure for a whole-picture operation; it "
            "is not the 64 MB localized-editing target, which is a different operation",
        },
        "cases": cases,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=1), encoding="utf-8")
    print(f"wrote {args.output} ({args.output.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
