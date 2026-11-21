#!/usr/bin/env python3
"""Measure what adaptive tiling does to the corpus.

The P5 corpus run measured every codec on the same canonical pixels and
compared the results with PNG, WebP and JPEG XL. This script asks a
narrower question: **does choosing the tile sizes from the picture's content
make the file smaller or bigger?**

The rules are the ones the corpus report already uses:

* the denominator is the **canonical pixel bytes**, the same number the P5
  report divides by, so the two reports can be read side by side;
* the median is the middle sample, or the mean of the two middle samples
  when the count is even;
* the p95 is the **nearest-rank** value;
* the standard deviation is the sample deviation (n-1);
* every image is checked for **exactness**: the adaptive file is decoded and
  compared with the original pixels, and a mismatch is a failure, not a
  footnote.

Run from the repository root:

    python benchmarks/corpus/run_adaptive.py
    python benchmarks/corpus/run_adaptive.py --category photos --jobs 2
"""

import argparse
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
sys.path.insert(0, str(REPO / "benchmarks" / "stress"))
from canonicalize import convert_image  # noqa: E402

#: The decimal byte ceiling the roadmap calls the secondary target.
SIZE_CEILING_BYTES = 10_000_000


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


def tile_count(path: Path) -> int:
    """The number of tiles a file's header declares."""
    with path.open("rb") as handle:
        handle.seek(28)
        return int.from_bytes(handle.read(4), "little")


def run_vpng(vpng: Path, args: list[str], timeout: int = 900) -> dict:
    started = time.perf_counter()
    proc = subprocess.run(
        [str(vpng), *args], capture_output=True, text=True, timeout=timeout
    )
    return {
        "exit": proc.returncode,
        "ms": round((time.perf_counter() - started) * 1000),
        "output": (proc.stdout + proc.stderr).strip()[:300],
    }


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


def measure_entry(
    entry: dict,
    root: Path,
    work: Path,
    vpng: Path,
    cache: dict[str, Path],
    tile_size: int,
    workers: int,
    keep_raw: bool = False,
) -> dict | None:
    source = root / entry["path"]
    if not source.exists():
        print(f"  missing image: {source}")
        return None

    work.mkdir(parents=True, exist_ok=True)
    raw = work / "canonical.raw"
    cached = cache.get(entry["sha256"])
    if cached is not None:
        raw = cached
        meta = {
            "width": entry["width"],
            "height": entry["height"],
            "pixel_format": entry["pixel_format"],
            "bytes": entry["canonical_bytes"],
            "sha256": entry["sha256"],
        }
    else:
        meta = convert_image(str(source), str(raw))

    width, height, fmt = meta["width"], meta["height"], meta["pixel_format"]
    base = [
        "--input-format", "raw",
        "--width", str(width),
        "--height", str(height),
        "--format", fmt,
        "--tile-size", str(tile_size),
        "--workers", str(workers),
        "--codec", "auto",
    ]

    record = {
        "id": entry["id"],
        "category": entry["category"],
        "synthetic": bool(entry.get("synthetic", False)),
        "width": width,
        "height": height,
        "long_edge": max(width, height),
        "pixel_format": fmt,
        "canonical_bytes": meta["bytes"],
        "canonical_sha256": meta["sha256"],
    }

    uniform_path = work / "uniform.vpng"
    adaptive_path = work / "adaptive.vpng"

    uniform = run_vpng(vpng, ["encode", str(raw), str(uniform_path), *base])
    if uniform["exit"] != 0:
        record["uniform"] = {"error": uniform["output"], "exit": uniform["exit"]}
    else:
        size = uniform_path.stat().st_size
        record["uniform"] = {
            "bytes": size,
            "ratio": round(size / meta["bytes"], 6),
            "tiles": tile_count(uniform_path),
            "ms": uniform["ms"],
        }

    adaptive = run_vpng(vpng, ["encode", str(raw), str(adaptive_path), *base, "--adaptive"])
    if adaptive["exit"] != 0:
        record["adaptive"] = {"error": adaptive["output"], "exit": adaptive["exit"]}
    else:
        size = adaptive_path.stat().st_size
        check = run_vpng(vpng, ["check", str(adaptive_path)])
        # The exactness check: the file is decoded and compared with the
        # original pixels. A mismatch fails the image. The original is the
        # raw canonical buffer, so it is read with the same geometry and
        # format the encoder was given.
        verify = run_vpng(
            vpng,
            ["verify", str(raw), str(adaptive_path), *base[:8]],
        )
        record["adaptive"] = {
            "bytes": size,
            "ratio": round(size / meta["bytes"], 6),
            "tiles": tile_count(adaptive_path),
            "ms": adaptive["ms"],
            "check_exit": check["exit"],
            "verify_exit": verify["exit"],
            "exact": verify["exit"] == 0,
        }

    if "bytes" in record.get("uniform", {}) and "bytes" in record.get("adaptive", {}):
        delta = record["adaptive"]["bytes"] - record["uniform"]["bytes"]
        record["delta_bytes"] = delta
        record["delta_ratio"] = round(delta / meta["bytes"], 6)
        record["adaptive_smaller"] = delta < 0
        record["adaptive_larger"] = delta > 0
        record["adaptive_equal"] = delta == 0
        record["under_10mb_uniform"] = record["uniform"]["bytes"] <= SIZE_CEILING_BYTES
        record["under_10mb_adaptive"] = record["adaptive"]["bytes"] <= SIZE_CEILING_BYTES

    uniform_path.unlink(missing_ok=True)
    adaptive_path.unlink(missing_ok=True)
    if not keep_raw and cached is None:
        raw.unlink(missing_ok=True)
    try:
        work.rmdir()
    except OSError:
        pass
    return record


def summarise(records: list[dict]) -> dict:
    """The distribution the report quotes."""

    def bucket(items: list[dict]) -> dict:
        row: dict = {"count": len(items)}
        for name in ("uniform", "adaptive"):
            sizes = [
                item[name]["bytes"]
                for item in items
                if "bytes" in item.get(name, {})
            ]
            ratios = [
                item[name]["ratio"]
                for item in items
                if "ratio" in item.get(name, {})
            ]
            tiles = [
                item[name]["tiles"]
                for item in items
                if "tiles" in item.get(name, {})
            ]
            row[name] = {
                "bytes": distribution([float(value) for value in sizes]),
                "ratio": distribution([float(value) for value in ratios]),
                "tiles": distribution([float(value) for value in tiles]),
                "measured": len(sizes),
            }
        deltas = [item["delta_bytes"] for item in items if "delta_bytes" in item]
        row["delta_bytes"] = distribution([float(value) for value in deltas])
        row["delta_ratio"] = distribution(
            [float(item["delta_ratio"]) for item in items if "delta_ratio" in item]
        )
        row["adaptive_smaller"] = sum(1 for item in items if item.get("adaptive_smaller"))
        row["adaptive_larger"] = sum(1 for item in items if item.get("adaptive_larger"))
        row["adaptive_equal"] = sum(1 for item in items if item.get("adaptive_equal"))
        row["exact"] = sum(
            1
            for item in items
            if item.get("adaptive", {}).get("exact") is True
        )
        row["check_failures"] = sum(
            1
            for item in items
            if item.get("adaptive", {}).get("check_exit", 0) != 0
        )
        row["under_10mb_uniform"] = sum(1 for item in items if item.get("under_10mb_uniform"))
        row["under_10mb_adaptive"] = sum(1 for item in items if item.get("under_10mb_adaptive"))
        row["total_uniform_bytes"] = sum(
            item["uniform"]["bytes"] for item in items if "bytes" in item.get("uniform", {})
        )
        row["total_adaptive_bytes"] = sum(
            item["adaptive"]["bytes"] for item in items if "bytes" in item.get("adaptive", {})
        )
        row["total_canonical_bytes"] = sum(item["canonical_bytes"] for item in items)
        return row

    categories: dict[str, list[dict]] = {}
    for record in records:
        categories.setdefault(record["category"], []).append(record)

    return {
        "total_images": len(records),
        "categories": {name: bucket(items) for name, items in sorted(categories.items())},
        "overall": bucket(records),
    }


def markdown(records: list[dict], summary: dict, context: dict) -> str:
    lines = [
        "# Adaptive tiling over the corpus",
        "",
        f"Generated: {context['generated_at']}",
        f"VPNG: `{context['vpng']}`",
        "",
        "Ratios are encoded size divided by canonical pixel bytes on the same pixels.",
        "",
        "| id | category | dimensions | canonical | uniform | adaptive | delta | exact |",
        "|---|---|---|---:|---:|---:|---:|---|",
    ]
    for record in records:
        uniform = record.get("uniform", {})
        adaptive = record.get("adaptive", {})
        lines.append(
            "| {id} | {category} | {width}x{height} | {canonical:,} | {uniform} | {adaptive} | "
            "{delta} | {exact} |".format(
                id=record["id"],
                category=record["category"],
                width=record["width"],
                height=record["height"],
                canonical=record["canonical_bytes"],
                uniform=f"{uniform['bytes']:,}" if "bytes" in uniform else "-",
                adaptive=f"{adaptive['bytes']:,}" if "bytes" in adaptive else "-",
                delta=f"{record['delta_bytes']:+,}" if "delta_bytes" in record else "-",
                exact="yes" if adaptive.get("exact") else "no",
            )
        )

    lines += [
        "",
        "## Category medians",
        "",
        "| category | images | uniform | adaptive | median delta | smaller | larger | equal | exact |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for category, row in summary["categories"].items():
        def med(name):
            stats = row.get(name, {}).get("ratio", {})
            value = stats.get("median")
            return "-" if value is None else f"{value:.3f}x"

        delta = row["delta_bytes"].get("median")
        lines.append(
            f"| {category} | {row['count']} | {med('uniform')} | {med('adaptive')} | "
            f"{'-' if delta is None else format(delta, ',.0f')} | "
            f"{row['adaptive_smaller']} | {row['adaptive_larger']} | {row['adaptive_equal']} | "
            f"{row['exact']}/{row['count']} |"
        )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(REPO / "corpus-data"))
    parser.add_argument("--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json"))
    parser.add_argument("--vpng", default=str(REPO / "target" / "release" / "vpng.exe"))
    parser.add_argument("--cache-root", default=str(REPO / "corpus-data" / "cache"))
    parser.add_argument("--results", default=str(REPO / "benchmarks" / "results"))
    parser.add_argument("--output", default=None, help="also write the JSON report here")
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--category", action="append", default=[])
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--jobs", type=int, default=1, help="images measured at the same time")
    args = parser.parse_args()

    vpng = Path(args.vpng)
    if not vpng.exists():
        raise SystemExit(f"vpng binary not found: {vpng} (cargo build --release --workspace)")
    root = Path(args.root)
    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    entries = manifest.get("images", [])
    if args.only:
        entries = [entry for entry in entries if entry["id"] in args.only]
    if args.category:
        entries = [entry for entry in entries if entry["category"] in args.category]
    if not entries:
        raise SystemExit("no manifest entries selected")

    results_dir = Path(args.results)
    results_dir.mkdir(parents=True, exist_ok=True)
    work_root = results_dir / "adaptive-work"
    cache = cache_index(Path(args.cache_root))

    started = time.perf_counter()
    records: list[dict] = []

    def measure(entry: dict) -> dict | None:
        work = work_root / entry["id"]
        record = measure_entry(
            entry, root, work, vpng, cache, args.tile_size, args.workers
        )
        if record is None:
            return None
        uniform = record.get("uniform", {})
        adaptive = record.get("adaptive", {})
        print(
            "   canonical {:,}  uniform {}  adaptive {}  delta {:+}  exact {}".format(
                record["canonical_bytes"],
                f"{uniform['bytes']:,}" if "bytes" in uniform else "failed",
                f"{adaptive['bytes']:,}" if "bytes" in adaptive else "failed",
                record.get("delta_bytes", 0),
                adaptive.get("exact"),
            )
        )
        return record

    if args.jobs > 1:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            for record in pool.map(measure, entries):
                if record is not None:
                    records.append(record)
    else:
        for entry in entries:
            print(f"== {entry['id']} ({entry['category']})")
            record = measure(entry)
            if record is not None:
                records.append(record)

    stamp = time.strftime("%Y%m%d_%H%M%S")
    summary = summarise(records)
    context = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "vpng": str(vpng),
        "python": sys.version.split()[0],
        "parameters": {
            "tile_size": args.tile_size,
            "workers": args.workers,
            "jobs": args.jobs,
            "codec": "auto",
            "denominator": "canonical pixel bytes",
            "size_ceiling_bytes": SIZE_CEILING_BYTES,
        },
        "elapsed_seconds": round(time.perf_counter() - started, 3),
    }
    report = {"context": context, "summary": summary, "images": records}
    json_path = results_dir / f"adaptive_{stamp}.json"
    md_path = results_dir / f"adaptive_{stamp}.md"
    json_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    md_path.write_text(markdown(records, summary, context), encoding="utf-8")
    if args.output:
        output = Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    overall = summary["overall"]
    print()
    print(
        "uniform {:,} bytes  adaptive {:,} bytes  delta {:+,} bytes".format(
            overall["total_uniform_bytes"],
            overall["total_adaptive_bytes"],
            overall["total_adaptive_bytes"] - overall["total_uniform_bytes"],
        )
    )
    print(
        "smaller {}  larger {}  equal {}  exact {}/{}".format(
            overall["adaptive_smaller"],
            overall["adaptive_larger"],
            overall["adaptive_equal"],
            overall["exact"],
            overall["count"],
        )
    )
    print(f"report: {json_path}")
    if args.output:
        print(f"stable output: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
