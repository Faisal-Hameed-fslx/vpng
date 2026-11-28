#!/usr/bin/env python3
"""Measure the v0.5 GPU payload over the 750-image corpus.

The format revision v0.5 adds an **optional GPU payload**: a file may carry,
beside every tile's lossless payload, a second, explicitly named, **lossy**
payload in the block format a GPU samples as a texture (BC1 colour blocks,
BC4 alpha blocks).

The GPU payload is a **sibling** of the lossless payload, never a
replacement for it. This script measures that claim over the same corpus
every other milestone used:

* **The lossless payload is untouched.** The plain v0.1 file and the GPU
  v0.5 file are built from the same pixels, and the v0.5 file's decoded
  pixels are compared with the canonical pixels, byte for byte, for every
  image.
* **The GPU payload is a different picture.** The v0.5 file's GPU payload is
  decoded to RGBA8 and compared with the original, pixel by pixel: the
  peak signal-to-noise ratio, the mean absolute error and the worst single
  difference are all measured.
* **Both payloads are accounted for.** The file's size, the GPU table's
  size and the GPU payloads' size are read back out of the file, so the
  trade the GPU payload makes is a measured number and not an estimate.
* **The file is a v0.5 file.** Every one of them says so in its minor
  version, and every one of them passes the integrity check.

Nothing here is a claim about speed on a GPU. There is no GPU in this
measurement: the script measures bytes, equality and fidelity, and it
reports what it found.

Run from the repository root:

    python benchmarks/corpus/run_gpu_corpus.py
    python benchmarks/corpus/run_gpu_corpus.py --category photos --jobs 1
"""

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

#: The decimal byte ceiling the roadmap calls the secondary target.
SIZE_CEILING_BYTES = 10_000_000

#: How many bytes one pixel takes, and how many of them are the colour
#: channels (the rest, if any, are the alpha channel).
CHANNELS = {
    "gray8": (1, 1),
    "graya8": (2, 1),
    "rgb8": (3, 3),
    "rgba8": (4, 3),
}


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
    """The SHA-256 of a file's bytes."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
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
        if digest and name:
            raw = cache_root / name
            if raw.exists():
                index[digest] = raw
    return index


def run_vpng(vpng: Path, args: list[str], timeout: int = 7200) -> dict:
    """Run a `vpng` subcommand and report what it did."""
    started = time.perf_counter()
    proc = subprocess.run(
        [str(vpng), *args], capture_output=True, text=True, timeout=timeout
    )
    return {
        "exit": proc.returncode,
        "ms": round((time.perf_counter() - started) * 1000),
        # The full standard output, for the commands whose JSON is read.
        "stdout": proc.stdout,
        # A short form of everything the command said, for the error
        # messages.
        "output": (proc.stdout + proc.stderr).strip()[:400],
    }


def run_with_retry(vpng: Path, args: list[str], what: str, attempts: int = 3) -> dict:
    """Run a `vpng` subcommand, retrying an allocation failure.

    The host is memory-pressured, so a large encode can fail outright with
    an allocation error. A retry clears the transient pressure. The retries
    are counted and reported, never hidden.
    """
    tries = 0
    while True:
        tries += 1
        result = run_vpng(vpng, args)
        if result["exit"] == 0:
            result["attempts"] = tries
            return result
        if tries >= attempts:
            result["attempts"] = tries
            raise RuntimeError(
                f"vpng {what} failed after {tries} attempts "
                f"(exit {result['exit']}): {result['output']}"
            )
        print(
            f"   attempt {tries} failed for {what} (exit {result['exit']}); retrying",
            flush=True,
        )


def read_json_output(result: dict) -> dict:
    """Read the JSON a `vpng --json` command printed."""
    text = result["stdout"]
    # The command may print a human line before the JSON; the JSON object is
    # the last line that starts with a brace.
    for line in reversed(text.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                continue
    raise RuntimeError(f"no JSON in the command's output: {text[:200]}")


def compare_fidelity(
    original: Path, gpu: Path, width: int, height: int, fmt: str
) -> dict:
    """Compare the GPU payload with the original, a row at a time.

    The GPU payload decodes to RGBA8 whatever the original's format is, so
    the original is widened to RGBA8 as it is read. The comparison never
    holds more than one row of either file in memory, which is what lets the
    16K image be measured on a host with a few gigabytes of RAM.

    The statistics are over **every** channel, the alpha channel included:
    for a format without one, the original's implicit alpha is 255, and the
    GPU payload's alpha is 255 too.
    """
    bpp, colour = CHANNELS[fmt]
    has_alpha = bpp > colour
    row_bytes = width * bpp
    gpu_row_bytes = width * 4

    squared_error = 0
    absolute_error = 0
    worst = 0
    samples = 0

    with original.open("rb") as source, gpu.open("rb") as decoded:
        for y in range(height):
            row = source.read(row_bytes)
            if len(row) < row_bytes:
                raise RuntimeError(f"{original} ended early at row {y}")
            gpu_row = decoded.read(gpu_row_bytes)
            if len(gpu_row) < gpu_row_bytes:
                raise RuntimeError(f"{gpu} ended early at row {y}")

            for x in range(width):
                at = x * bpp
                if fmt == "gray8":
                    grey = row[at]
                    red = green = blue = grey
                    alpha = 255
                elif fmt == "graya8":
                    grey = row[at]
                    red = green = blue = grey
                    alpha = row[at + 1]
                elif fmt == "rgb8":
                    red, green, blue = row[at], row[at + 1], row[at + 2]
                    alpha = 255
                else:
                    red, green, blue, alpha = (
                        row[at],
                        row[at + 1],
                        row[at + 2],
                        row[at + 3],
                    )

                gat = x * 4
                pairs = (
                    (red, gpu_row[gat]),
                    (green, gpu_row[gat + 1]),
                    (blue, gpu_row[gat + 2]),
                    (alpha, gpu_row[gat + 3]),
                )
                for left, right in pairs:
                    difference = abs(left - right)
                    squared_error += difference * difference
                    absolute_error += difference
                    if difference > worst:
                        worst = difference
                    samples += 1

    mse = squared_error / samples if samples else 0.0
    psnr = 10.0 * math.log10((255.0 * 255.0) / mse) if mse > 0 else None
    return {
        "samples": samples,
        "mse": round(mse, 6),
        "psnr_db": round(psnr, 4) if psnr is not None else None,
        "mean_absolute_error": round(absolute_error / samples, 6)
        if samples
        else None,
        "max_absolute_error": worst,
        "has_alpha": has_alpha,
    }


def measure_entry(
    entry: dict,
    work: Path,
    vpng: Path,
    cache: dict[str, Path],
    tile_size: int,
    workers: int,
    keep: bool,
) -> dict | None:
    """Build the plain and the GPU file for one corpus image and measure them."""
    raw = cache.get(entry["sha256"])
    if raw is None:
        print(f"  no cached canonical pixels for {entry['id']}")
        return None

    width, height = entry["width"], entry["height"]
    fmt = entry["pixel_format"]

    image_work = work / entry["id"]
    image_work.mkdir(parents=True, exist_ok=True)

    plain = image_work / "plain.vpng"
    gpu_file = image_work / "gpu.vpng"
    exact_raw = image_work / "exact.raw"
    gpu_raw = image_work / "gpu-payload.raw"

    record = {
        "id": entry["id"],
        "category": entry["category"],
        "synthetic": bool(entry.get("synthetic", False)),
        "origin": entry.get("origin", "unknown"),
        "width": width,
        "height": height,
        "pixel_format": fmt,
        "canonical_bytes": entry["canonical_bytes"],
        "canonical_sha256": entry["sha256"],
    }

    # 1. The plain v0.1 file: the baseline the GPU table's cost is measured
    # against.
    plain_result = run_with_retry(
        vpng,
        [
            "encode",
            str(raw),
            str(plain),
            "--input-format",
            "raw",
            "--width",
            str(width),
            "--height",
            str(height),
            "--format",
            fmt,
            "--tile-size",
            str(tile_size),
            "--workers",
            str(workers),
        ],
        "encode",
    )
    record["plain_bytes"] = plain.stat().st_size
    record["plain_sha256"] = file_sha256(plain)
    record["plain_ms"] = plain_result["ms"]

    # 2. The GPU file: the same pixels, plus a GPU payload beside every
    # tile's lossless payload.
    gpu_result = run_with_retry(
        vpng,
        [
            "encode",
            str(raw),
            str(gpu_file),
            "--input-format",
            "raw",
            "--width",
            str(width),
            "--height",
            str(height),
            "--format",
            fmt,
            "--tile-size",
            str(tile_size),
            "--workers",
            str(workers),
            "--gpu",
        ],
        "encode --gpu",
    )
    record["gpu_file_bytes"] = gpu_file.stat().st_size
    record["gpu_file_sha256"] = file_sha256(gpu_file)
    record["gpu_encode_ms"] = gpu_result["ms"]
    record["gpu_encode_attempts"] = gpu_result["attempts"]

    check = run_with_retry(vpng, ["check", str(gpu_file)], "check (gpu)")
    record["gpu_check_ok"] = check["exit"] == 0

    info = read_json_output(
        run_with_retry(vpng, ["info", str(gpu_file), "--json"], "info (gpu)")
    )
    record["gpu_minor"] = info["minor"]
    record["gpu_table_offset"] = info["gpu_table_offset"]
    record["gpu_table_length"] = info["gpu_table_length"]
    record["gpu_tile_count"] = info["tile_count"]

    table = read_json_output(
        run_with_retry(vpng, ["gpu-info", str(gpu_file), "--json"], "gpu-info (gpu)")
    )
    record["gpu_record_count"] = table["record_count"]
    record["gpu_payload_bytes"] = table["gpu_payload_bytes"]
    record["gpu_tiles_with_colour"] = table["tiles_with_colour"]
    record["gpu_tiles_with_alpha"] = table["tiles_with_alpha"]

    # 3. The lossless payload still reconstructs the picture exactly.
    run_with_retry(
        vpng,
        ["decode", str(gpu_file), str(exact_raw), "--format", "raw"],
        "decode (gpu)",
    )
    record["lossless_exact"] = file_sha256(exact_raw) == entry["sha256"]

    # 4. The GPU payload is a different picture: its fidelity is measured.
    run_with_retry(
        vpng,
        [
            "gpu-decode",
            str(gpu_file),
            str(gpu_raw),
            "--format",
            "raw",
            "--x",
            "0",
            "--y",
            "0",
            "--width",
            str(width),
            "--height",
            str(height),
        ],
        "gpu-decode (gpu)",
    )
    record["fidelity"] = compare_fidelity(raw, gpu_raw, width, height, fmt)
    record["gpu_payload_differs"] = (
        record["fidelity"]["max_absolute_error"] > 0
    )

    # 5. The bytes add up: the GPU file is the plain file plus the GPU
    # table, the GPU payloads, and the eight bytes the header grew by.
    record["gpu_overhead_bytes"] = record["gpu_file_bytes"] - record["plain_bytes"]
    record["gpu_overhead_ratio"] = (
        round(record["gpu_overhead_bytes"] / record["plain_bytes"], 8)
        if record["plain_bytes"]
        else None
    )
    record["bytes_add_up"] = (
        record["gpu_overhead_bytes"]
        == record["gpu_payload_bytes"] + record["gpu_table_length"] + 8
    )

    if not keep:
        for path in (plain, gpu_file, exact_raw, gpu_raw):
            if path.exists():
                path.unlink()
        if image_work.exists():
            for child in image_work.iterdir():
                if child.is_file():
                    child.unlink()
            image_work.rmdir()

    return record


def summarise(records: list[dict]) -> dict:
    """The summary the report quotes."""
    summary: dict = {"images": len(records)}

    def bucket(items: list[dict]) -> dict:
        if not items:
            return {"count": 0}
        out = {
            "count": len(items),
            "plain_bytes": sum(item["plain_bytes"] for item in items),
            "gpu_file_bytes": sum(item["gpu_file_bytes"] for item in items),
            "gpu_payload_bytes": sum(item["gpu_payload_bytes"] for item in items),
            "gpu_table_bytes": sum(item["gpu_table_length"] for item in items),
            "canonical_bytes": sum(item["canonical_bytes"] for item in items),
            "lossless_exact": sum(1 for item in items if item["lossless_exact"]),
            "check_failures": sum(
                1 for item in items if not item.get("gpu_check_ok", False)
            ),
            "bytes_add_up_failures": sum(
                1 for item in items if not item.get("bytes_add_up", False)
            ),
            "gpu_payload_differs": sum(
                1 for item in items if item.get("gpu_payload_differs", False)
            ),
            "minor_versions": sorted({item["gpu_minor"] for item in items}),
            "plain_under_ceiling": sum(
                1 for item in items if item["plain_bytes"] <= SIZE_CEILING_BYTES
            ),
            "gpu_under_ceiling": sum(
                1 for item in items if item["gpu_file_bytes"] <= SIZE_CEILING_BYTES
            ),
        }
        out["gpu_overhead_bytes"] = out["gpu_file_bytes"] - out["plain_bytes"]
        out["gpu_overhead_ratio"] = (
            round(out["gpu_overhead_bytes"] / out["plain_bytes"], 8)
            if out["plain_bytes"]
            else None
        )
        out["gpu_payload_ratio_to_canonical"] = (
            round(out["gpu_payload_bytes"] / out["canonical_bytes"], 8)
            if out["canonical_bytes"]
            else None
        )

        # The fidelity, pooled over every pixel of every image in the bucket.
        squared = 0
        absolute = 0
        samples = 0
        worst = 0
        for item in items:
            fidelity = item["fidelity"]
            squared += fidelity["mse"] * fidelity["samples"]
            absolute += fidelity["mean_absolute_error"] * fidelity["samples"]
            samples += fidelity["samples"]
            worst = max(worst, fidelity["max_absolute_error"])
        mse = squared / samples if samples else 0.0
        psnr = 10.0 * math.log10((255.0 * 255.0) / mse) if mse > 0 else None
        out["fidelity"] = {
            "samples": samples,
            "mse": round(mse, 6),
            "psnr_db": round(psnr, 4) if psnr is not None else None,
            "mean_absolute_error": round(absolute / samples, 6)
            if samples
            else None,
            "max_absolute_error": worst,
        }
        out["psnr_distribution"] = distribution(
            [
                float(item["fidelity"]["psnr_db"])
                for item in items
                if item["fidelity"]["psnr_db"] is not None
            ]
        )
        out["max_error_distribution"] = distribution(
            [float(item["fidelity"]["max_absolute_error"]) for item in items]
        )
        return out

    summary["overall"] = bucket(records)
    by_category: dict[str, list[dict]] = {}
    for record in records:
        by_category.setdefault(record["category"], []).append(record)
    summary["by_category"] = {
        name: bucket(items) for name, items in sorted(by_category.items())
    }

    # The encode time: the GPU payload costs time as well as bytes.
    summary["encode_ms"] = {
        "plain": distribution([float(item["plain_ms"]) for item in records]),
        "gpu": distribution([float(item["gpu_encode_ms"]) for item in records]),
    }
    plain_total = sum(item["plain_ms"] for item in records)
    gpu_total = sum(item["gpu_encode_ms"] for item in records)
    summary["encode_ms"]["gpu_over_plain"] = (
        round(gpu_total / plain_total, 4) if plain_total else None
    )
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=REPO / "benchmarks/corpus_manifest.json")
    parser.add_argument("--cache", type=Path, default=REPO / "corpus-data/cache")
    parser.add_argument("--vpng", type=Path, default=REPO / "target/release/vpng.exe")
    parser.add_argument("--work", type=Path, default=REPO / "benchmarks/work/gpu")
    parser.add_argument("--output", type=Path, default=REPO / "benchmarks/p13_gpu_results.json")
    parser.add_argument("--category", default=None, help="measure one category only")
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--jobs", type=int, default=1, help="images measured at once")
    parser.add_argument("--limit", type=int, default=0, help="measure only the first N images")
    parser.add_argument("--keep", action="store_true", help="keep the files that were built")
    args = parser.parse_args()

    if not args.vpng.exists():
        print(f"no vpng binary at {args.vpng}; run `cargo build --release -p vpng-cli`")
        return 2

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    images = manifest["images"]
    if args.category:
        images = [item for item in images if item["category"] == args.category]
    if args.limit:
        images = images[: args.limit]

    cache = cache_index(args.cache)
    print(f"{len(images)} image(s); {len(cache)} cached canonical file(s)")

    args.work.mkdir(parents=True, exist_ok=True)
    started = time.perf_counter()
    records: list[dict] = []

    def one(entry: dict) -> dict | None:
        try:
            return measure_entry(
                entry, args.work, args.vpng, cache, args.tile_size, args.workers, args.keep
            )
        except Exception as error:  # noqa: BLE001
            print(f"  {entry['id']}: {error}")
            return None

    if args.jobs > 1:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            for index, result in enumerate(pool.map(one, images), start=1):
                if result is not None:
                    records.append(result)
                print(f"  [{index}/{len(images)}] {result['id'] if result else 'skipped'}")
    else:
        for index, entry in enumerate(images, start=1):
            result = one(entry)
            if result is not None:
                records.append(result)
            print(f"  [{index}/{len(images)}] {entry['id']}", flush=True)

    elapsed = time.perf_counter() - started
    summary = summarise(records)
    payload = {
        "schema": "vpng.gpu.corpus.v1",
        "generated_by": "benchmarks/corpus/run_gpu_corpus.py",
        "format_minor": 5,
        "tile_size": args.tile_size,
        "workers": args.workers,
        "jobs": args.jobs,
        "elapsed_seconds": round(elapsed, 3),
        "summary": summary,
        "records": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=1), encoding="utf-8")
    print(f"wrote {args.output} ({args.output.stat().st_size} bytes)")
    print(json.dumps(summary["overall"], indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
