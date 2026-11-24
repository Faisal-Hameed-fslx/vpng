#!/usr/bin/env python3
"""Measure the production encoder's two codec sets over the 750-image corpus.

The research harness (`run_codec_research.py`) answers what each candidate
*would* cost. This script answers the question that decides whether the
promotion is safe to ship: **what the production encoder actually writes**.

For every corpus image it runs the real `vpng` binary twice, on the same
canonical pixels and the same tile layout:

* `--codec-set stable` — the four original codecs (ids 0-3). This is the
  default path, and its bytes must not move.
* `--codec-set extended` — those four plus the three promoted in v0.3
  (ids 4-6).

Each file is then:

* checked with `vpng check` (header and directory checksums);
* **verified** with `vpng verify`, which decodes every tile and compares the
  result with the original pixels — so a file that is smaller but not
  lossless fails the run rather than being reported as a win;
* digested with SHA-256, so two runs can be compared byte for byte.

The stable run is also compared against the P5 corpus baseline, which is the
proof that the promotion did not move the published bytes.

Run from the repository root:

    python benchmarks/corpus/run_codec_corpus.py
    python benchmarks/corpus/run_codec_corpus.py --category photos --jobs 1
"""

import argparse
import hashlib
import json
import math
import statistics
import struct
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

#: The decimal byte ceiling the roadmap calls the secondary target.
SIZE_CEILING_BYTES = 10_000_000

#: The codec ids the two sets may name.
STABLE_IDS = (0, 1, 2, 3)
EXTENDED_IDS = (0, 1, 2, 3, 4, 5, 6)

#: The human-readable names of the stable codec ids.
CODEC_NAMES = {
    0: "raw",
    1: "palette",
    2: "rle",
    3: "predictor",
    4: "lz77",
    5: "predictor+dictionary",
    6: "context-entropy",
}

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
        if digest and name:
            raw = cache_root / name
            if raw.exists():
                index[digest] = raw
    return index


def read_header(path: Path) -> dict:
    """Read the header fields the report quotes.

    The layout is the one the specification defines: a 64-byte header, a
    40-byte directory entry per tile, and the codec id at offset 12 of an
    entry.
    """
    with path.open("rb") as handle:
        head = handle.read(64)
        if len(head) < 64:
            raise ValueError(f"{path} is shorter than a header")
        major, minor = struct.unpack("<HH", head[8:12])
        (tile_count,) = struct.unpack("<I", head[28:32])
        (directory_offset,) = struct.unpack("<Q", head[32:40])
        (feature_flags,) = struct.unpack("<I", head[48:52])
        histogram: dict[str, int] = {}
        for index in range(tile_count):
            handle.seek(directory_offset + index * 40 + 12)
            codec = handle.read(1)[0]
            name = CODEC_NAMES.get(codec, f"unknown-{codec}")
            histogram[name] = histogram.get(name, 0) + 1
    return {
        "major": major,
        "minor": minor,
        "tile_count": tile_count,
        "feature_flags": feature_flags,
        "tiles_by_codec": histogram,
    }


def run_vpng(vpng: Path, args: list[str], timeout: int = 7200) -> dict:
    started = time.perf_counter()
    proc = subprocess.run(
        [str(vpng), *args], capture_output=True, text=True, timeout=timeout
    )
    return {
        "exit": proc.returncode,
        "ms": round((time.perf_counter() - started) * 1000),
        "output": (proc.stdout + proc.stderr).strip()[:400],
    }


def run_with_retry(vpng: Path, args: list[str], what: str, attempts: int = 3) -> dict:
    """Run a `vpng` subcommand, retrying an allocation failure.

    The host is memory-pressured, so a large encode can fail outright with an
    allocation error. A retry clears the transient pressure. The retries are
    counted and reported, never hidden.
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


def measure_entry(
    entry: dict,
    root: Path,
    work: Path,
    vpng: Path,
    cache: dict[str, Path],
    tile_size: int,
    workers: int,
    baseline: dict,
    determinism_workers: int | None,
) -> dict | None:
    """Encode one corpus image with both codec sets and verify both files."""
    source = root / entry["path"]
    if not source.exists():
        print(f"  missing image: {source}")
        return None

    work.mkdir(parents=True, exist_ok=True)
    raw = cache.get(entry["sha256"])
    if raw is None:
        print(f"  no cached canonical pixels for {entry['id']}")
        return None

    width, height = entry["width"], entry["height"]
    fmt = entry["pixel_format"]
    canonical = entry["canonical_bytes"]
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
        str(tile_size),
        "--workers",
        str(workers),
    ]

    record = {
        "id": entry["id"],
        "category": entry["category"],
        "synthetic": bool(entry.get("synthetic", False)),
        "width": width,
        "height": height,
        "pixel_format": fmt,
        "canonical_bytes": canonical,
        "canonical_sha256": entry["sha256"],
    }

    for label, codec_set in (("stable", "stable"), ("extended", "extended")):
        out = work / f"{label}.vpng"
        encode = run_with_retry(
            vpng,
            ["encode", str(raw), str(out), *base, "--codec-set", codec_set],
            f"encode ({label})",
        )
        size = out.stat().st_size
        header = read_header(out)
        digest = file_sha256(out)
        check = run_with_retry(vpng, ["check", str(out)], f"check ({label})")
        verify = run_with_retry(
            vpng, ["verify", str(raw), str(out), *base], f"verify ({label})"
        )
        record[label] = {
            "bytes": size,
            "ratio": round(size / canonical, 6),
            "encode_ms": encode["ms"],
            "encode_attempts": encode["attempts"],
            "sha256": digest,
            "header": header,
            "check_exit": check["exit"],
            "verify_exit": verify["exit"],
            "verify_output": verify["output"],
            "under_10mb": size <= SIZE_CEILING_BYTES,
        }
        out.unlink(missing_ok=True)

    # The stable path must not have moved: the P5 corpus run measured the same
    # bytes with the default codec set.
    p5 = baseline.get(entry["id"])
    if p5 is not None:
        p5_bytes = p5.get("vpng", {}).get("auto", {}).get("bytes")
        record["p5_auto_bytes"] = p5_bytes
        record["stable_matches_p5"] = p5_bytes == record["stable"]["bytes"]

    # A spot check that the worker count does not move the bytes: the same
    # file encoded with a different worker count must have the same digest.
    if determinism_workers:
        out = work / "determinism.vpng"
        alt = list(base)
        alt[alt.index("--workers") + 1] = str(determinism_workers)
        run_with_retry(
            vpng,
            ["encode", str(raw), str(out), *alt, "--codec-set", "extended"],
            "encode (determinism)",
        )
        record["extended_alt_workers"] = determinism_workers
        record["extended_alt_workers_sha256"] = file_sha256(out)
        record["extended_deterministic"] = (
            record["extended_alt_workers_sha256"] == record["extended"]["sha256"]
        )
        out.unlink(missing_ok=True)

    stable = record["stable"]["bytes"]
    extended = record["extended"]["bytes"]
    record["delta_bytes"] = extended - stable
    record["extended_smaller"] = extended < stable
    record["extended_larger"] = extended > stable
    record["extended_equal"] = extended == stable

    try:
        work.rmdir()
    except OSError:
        pass
    return record


def summarise(records: list[dict]) -> dict:
    """The distribution the report quotes."""

    def bucket(items: list[dict]) -> dict:
        row: dict = {"count": len(items)}
        for name in ("stable", "extended"):
            sizes = [item[name]["bytes"] for item in items]
            ratios = [item[name]["ratio"] for item in items]
            row[name] = {
                "total_bytes": sum(sizes),
                "ratio": distribution(ratios),
                "under_10mb": sum(1 for item in items if item[name]["under_10mb"]),
                "check_failures": sum(1 for item in items if item[name]["check_exit"] != 0),
                "verify_failures": sum(1 for item in items if item[name]["verify_exit"] != 0),
            }
        row["delta_bytes"] = distribution([item["delta_bytes"] for item in items])
        row["extended_smaller"] = sum(1 for item in items if item["extended_smaller"])
        row["extended_larger"] = sum(1 for item in items if item["extended_larger"])
        row["extended_equal"] = sum(1 for item in items if item["extended_equal"])
        # Which codec each set chose, and how many files said what version.
        for name in ("stable", "extended"):
            histogram: dict[str, int] = {}
            minors: dict[str, int] = {}
            for item in items:
                for codec, count in item[name]["header"]["tiles_by_codec"].items():
                    histogram[codec] = histogram.get(codec, 0) + count
                key = str(item[name]["header"]["minor"])
                minors[key] = minors.get(key, 0) + 1
            row[name]["tiles_by_codec"] = histogram
            row[name]["files_by_minor"] = minors
        compared = [item for item in items if "stable_matches_p5" in item]
        row["compared_with_p5"] = len(compared)
        row["stable_matches_p5"] = sum(
            1 for item in compared if item["stable_matches_p5"]
        )
        deterministic = [item for item in items if "extended_deterministic" in item]
        row["determinism_checked"] = len(deterministic)
        row["determinism_failures"] = sum(
            1 for item in deterministic if not item["extended_deterministic"]
        )
        return row

    categories: dict[str, dict] = {}
    for name in sorted({record["category"] for record in records}):
        categories[name] = bucket(
            [record for record in records if record["category"] == name]
        )
    return {"overall": bucket(records), "categories": categories}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(REPO / "corpus-data"))
    parser.add_argument(
        "--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json")
    )
    parser.add_argument(
        "--vpng", default=str(REPO / "target" / "release" / "vpng.exe")
    )
    parser.add_argument(
        "--cache-root", default=str(REPO / "corpus-data" / "cache")
    )
    parser.add_argument(
        "--baseline",
        default=str(REPO / "benchmarks" / "p5_corpus_results.json"),
        help="the P5 corpus run the stable bytes are compared against",
    )
    parser.add_argument(
        "--output", default=str(REPO / "benchmarks" / "p8_codec_production_results.json")
    )
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--jobs", type=int, default=1, help="images measured at the same time")
    parser.add_argument("--category", action="append", default=[])
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument(
        "--determinism-samples",
        type=int,
        default=0,
        help=(
            "how many images to re-encode with one worker and compare digests "
            "against the main run"
        ),
    )
    args = parser.parse_args()

    vpng = Path(args.vpng)
    if not vpng.exists():
        raise SystemExit(
            f"vpng binary not found: {vpng} (cargo build --release --workspace)"
        )
    root = Path(args.root)
    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    entries = manifest.get("images", [])
    if args.only:
        entries = [entry for entry in entries if entry["id"] in args.only]
    if args.category:
        entries = [entry for entry in entries if entry["category"] in args.category]
    if not entries:
        raise SystemExit("no manifest entries selected")

    baseline_path = Path(args.baseline)
    baseline: dict[str, dict] = {}
    if baseline_path.exists():
        document = json.loads(baseline_path.read_text(encoding="utf-8"))
        baseline = {record["id"]: record for record in document.get("images", [])}

    cache = cache_index(Path(args.cache_root))
    work_root = REPO / "benchmarks" / "results" / "codec-corpus-work"
    samples = args.determinism_samples

    started = time.perf_counter()
    records: list[dict] = []

    def run(entry: dict, index: int) -> dict | None:
        return measure_entry(
            entry,
            root,
            work_root / entry["id"],
            vpng,
            cache,
            args.tile_size,
            args.workers,
            baseline,
            1 if (samples and index < samples) else None,
        )

    if args.jobs > 1:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = []
            for index, entry in enumerate(entries):
                print(f"== {entry['id']} ({entry['category']})", flush=True)
                futures.append(pool.submit(run, entry, index))
            for future in futures:
                record = future.result()
                if record is not None:
                    records.append(record)
                    _print_row(record)
    else:
        for index, entry in enumerate(entries):
            print(f"== {entry['id']} ({entry['category']})", flush=True)
            record = run(entry, index)
            if record is not None:
                records.append(record)
                _print_row(record)

    summary = summarise(records)
    document = {
        "tool": "run_codec_corpus.py",
        "schema": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "context": {
            "vpng": str(vpng),
            "manifest": str(args.manifest),
            "root": str(root),
            "baseline": str(baseline_path),
            "python": sys.version.split()[0],
            "jobs": args.jobs,
            "parameters": {
                "tile_size": args.tile_size,
                "workers": args.workers,
                "determinism_samples": samples,
                "stable_codec_ids": list(STABLE_IDS),
                "extended_codec_ids": list(EXTENDED_IDS),
                "denominator": "canonical pixel bytes",
                "size_ceiling_bytes": SIZE_CEILING_BYTES,
            },
        },
        "definitions": {
            "stable": (
                "the production encoder with `--codec-set stable`: the four "
                "original codecs, ids 0-3"
            ),
            "extended": (
                "the production encoder with `--codec-set extended`: those four "
                "plus the three promoted in v0.3, ids 4-6"
            ),
            "verify": (
                "`vpng verify` decodes every tile and compares the result with "
                "the original pixels; a non-zero exit is a lossless failure"
            ),
            "ratio": "file bytes divided by canonical pixel bytes; below 1.0 is smaller",
        },
        "summary": summary,
        "images": records,
        "elapsed_seconds": round(time.perf_counter() - started, 3),
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8")

    overall = summary["overall"]
    print()
    print(
        "stable {} bytes  extended {} bytes  delta {} bytes".format(
            f"{overall['stable']['total_bytes']:,}",
            f"{overall['extended']['total_bytes']:,}",
            f"{overall['extended']['total_bytes'] - overall['stable']['total_bytes']:,}",
        )
    )
    print(
        "smaller {}  larger {}  equal {}".format(
            overall["extended_smaller"],
            overall["extended_larger"],
            overall["extended_equal"],
        )
    )
    print(
        "check failures {}  verify failures {}".format(
            overall["stable"]["check_failures"] + overall["extended"]["check_failures"],
            overall["stable"]["verify_failures"] + overall["extended"]["verify_failures"],
        )
    )
    print(f"wrote {output}")
    return 0


def _print_row(record: dict) -> None:
    print(
        "   canonical {:>12,}  stable {:>12,}  extended {:>12,}  delta {:>+10,}  {}".format(
            record["canonical_bytes"],
            record["stable"]["bytes"],
            record["extended"]["bytes"],
            record["delta_bytes"],
            " ".join(
                f"{codec}={count}"
                for codec, count in sorted(
                    record["extended"]["header"]["tiles_by_codec"].items()
                )
            ),
        ),
        flush=True,
    )


if __name__ == "__main__":
    raise SystemExit(main())
