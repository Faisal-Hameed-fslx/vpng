#!/usr/bin/env python3
"""Run the corpus through VPNG and the available lossless baselines.

Reads `benchmarks/corpus_manifest.json`, canonicalises each image to raw
pixels (Pillow), encodes it with every implemented VPNG codec, and compares
sizes with PNG and WebP-lossless on the *same* canonical pixels. Results are
written to the git-ignored `benchmarks/results/` directory as JSON and
Markdown, and optionally to a stable path (`--output`) that a report can be
bound to.

The summary carries the distribution the roadmap asks for: per category and
per resolution, the median and p95 of the size and of the ratio, and the
share of images that land under the 10,000,000-byte ceiling.

Percentile rules match `crates/vpng-validation/src/photo_bench.rs`:

* the median is the middle sample, or the mean of the two middle samples
  when the count is even;
* the p95 is the **nearest-rank** value, so with ten samples it is the
  largest one;
* the standard deviation is the sample deviation (n-1).

Run from the repository root:
    python benchmarks/corpus/run_corpus.py
    python benchmarks/corpus/run_corpus.py --only user-photo-15360x8640
    python benchmarks/corpus/run_corpus.py --category photos --jobs 4
"""

import argparse
import json
import math
import statistics
import struct
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "benchmarks" / "stress"))
sys.path.insert(0, str(HERE))
from baselines import measure_baselines  # noqa: E402
from canonicalize import convert_image  # noqa: E402

CODECS = ["raw", "palette", "rle", "predictor", "auto"]

#: The decimal byte ceiling the roadmap calls the secondary target.
SIZE_CEILING_BYTES = 10_000_000

#: The resolution buckets the distribution is reported over.
RESOLUTION_BUCKETS = (
    ("<=512", 0, 512),
    ("513-1024", 513, 1024),
    ("1025-2048", 1025, 2048),
    ("2049-4096", 2049, 4096),
    ("4097-8192", 4097, 8192),
    (">8192", 8193, 1 << 30),
)


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


def codec_histogram(path: Path) -> dict:
    with path.open("rb") as fh:
        fh.seek(28)
        (tiles,) = struct.unpack("<I", fh.read(4))
        fh.seek(32)
        (directory_offset,) = struct.unpack("<Q", fh.read(8))
        histogram = {}
        for index in range(tiles):
            fh.seek(directory_offset + index * 40 + 12)
            codec = fh.read(1)[0]
            histogram[codec] = histogram.get(codec, 0) + 1
    return histogram


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


def measure_entry(
    entry: dict,
    root: Path,
    work: Path,
    vpng: Path,
    max_baseline_pixels: int,
    tile_size: int,
    workers: int,
    codecs: list[str],
) -> dict | None:
    source = root / entry["path"]
    if not source.exists():
        print(f"  missing image: {source}")
        return None

    work.mkdir(parents=True, exist_ok=True)
    raw = work / "canonical.raw"
    meta = convert_image(str(source), str(raw))
    sha_matches = meta["sha256"] == entry["sha256"]
    if not sha_matches:
        print(f"  SHA-256 MISMATCH for {entry['id']}: {meta['sha256']} != {entry['sha256']}")

    width, height, fmt = meta["width"], meta["height"], meta["pixel_format"]
    base = [
        "--input-format", "raw",
        "--width", str(width),
        "--height", str(height),
        "--format", fmt,
        "--tile-size", str(tile_size),
        "--workers", str(workers),
    ]

    record = {
        "id": entry["id"],
        "category": entry["category"],
        "synthetic": bool(entry.get("synthetic", False)),
        "origin": entry.get("origin", "unknown"),
        "width": width,
        "height": height,
        "long_edge": max(width, height),
        "pixel_format": fmt,
        "canonical_bytes": meta["bytes"],
        "canonical_sha256": meta["sha256"],
        "sha256_matches_manifest": sha_matches,
        "vpng": {},
        "baselines": {},
    }

    for codec in codecs:
        out = work / f"{codec}.vpng"
        result = run_vpng(vpng, ["encode", str(raw), str(out), *base, "--codec", codec])
        if result["exit"] == 0:
            size = out.stat().st_size
            check = run_vpng(vpng, ["check", str(out)])
            record["vpng"][codec] = {
                "bytes": size,
                "ratio": round(size / meta["bytes"], 5),
                "ms": result["ms"],
                "codecs": {str(k): v for k, v in codec_histogram(out).items()},
                "check_exit": check["exit"],
            }
        else:
            record["vpng"][codec] = {"error": result["output"], "exit": result["exit"]}
        out.unlink(missing_ok=True)

    baselines_ok = width * height <= max_baseline_pixels
    if baselines_ok:
        record["baselines"] = measure_baselines(
            raw, width, height, fmt, meta["bytes"], work, max_baseline_pixels
        )
    else:
        record["baselines"]["skipped"] = (
            f"{width * height} pixels exceeds the {max_baseline_pixels}-pixel baseline cap"
        )

    record["baselines"].setdefault("jxl", {"error": "no JPEG XL encoder installed"})
    record["baselines"].setdefault("flif", {"error": "no FLIF encoder installed"})

    forced = [
        record["vpng"][codec]["bytes"]
        for codec in ("raw", "palette", "rle", "predictor")
        if codec in record["vpng"] and "bytes" in record["vpng"][codec]
    ]
    if forced and "bytes" in record["vpng"].get("auto", {}):
        best = min(forced)
        auto = record["vpng"]["auto"]["bytes"]
        record["best_forced_bytes"] = best
        # Auto selection may mix codecs per tile, so it can beat every single-codec
        # run; it must never be larger than the best forced run.
        record["auto_no_worse_than_best"] = auto <= best
        record["auto_saved_bytes_vs_best"] = best - auto

    auto = record["vpng"].get("auto", {})
    record["under_10mb"] = "bytes" in auto and auto["bytes"] <= SIZE_CEILING_BYTES
    record["auto_bytes"] = auto.get("bytes")

    raw.unlink(missing_ok=True)
    try:
        work.rmdir()
    except OSError:
        pass
    return record


def summarise(records: list[dict], codecs: list[str]) -> dict:
    """The per-category and per-resolution distribution the report quotes."""

    def bucket_stats(items: list[dict]) -> dict:
        row: dict = {"count": len(items)}
        row["real"] = sum(1 for item in items if not item.get("synthetic"))
        row["synthetic"] = sum(1 for item in items if item.get("synthetic"))
        for codec in codecs:
            sizes = [
                item["vpng"][codec]["bytes"]
                for item in items
                if codec in item.get("vpng", {}) and "bytes" in item["vpng"][codec]
            ]
            ratios = [
                item["vpng"][codec]["ratio"]
                for item in items
                if codec in item.get("vpng", {}) and "bytes" in item["vpng"][codec]
            ]
            row[codec] = {
                "bytes": distribution([float(value) for value in sizes]),
                "ratio": distribution([float(value) for value in ratios]),
            }
        for tool in ("png", "webp", "jxl"):
            sizes = [
                item["baselines"][tool]["bytes"]
                for item in items
                if tool in item.get("baselines", {}) and "bytes" in item["baselines"][tool]
            ]
            ratios = [
                item["baselines"][tool]["ratio"]
                for item in items
                if tool in item.get("baselines", {}) and "bytes" in item["baselines"][tool]
            ]
            row[tool] = {
                "bytes": distribution([float(value) for value in sizes]),
                "ratio": distribution([float(value) for value in ratios]),
                "measured": len(sizes),
            }
        under = [item for item in items if item.get("under_10mb")]
        row["under_10mb"] = {
            "count": len(under),
            "total": len(items),
            "rate": round(len(under) / len(items), 6) if items else None,
            "ceiling_bytes": SIZE_CEILING_BYTES,
        }
        comparable = [item for item in items if item.get("auto_no_worse_than_best") is not None]
        row["auto_no_worse_than_best"] = sum(
            1 for item in comparable if item.get("auto_no_worse_than_best")
        )
        row["auto_comparable"] = len(comparable)
        row["total_auto_bytes"] = sum(
            item["vpng"]["auto"]["bytes"]
            for item in items
            if "bytes" in item.get("vpng", {}).get("auto", {})
        )
        row["total_canonical_bytes"] = sum(item["canonical_bytes"] for item in items)
        return row

    categories: dict[str, list[dict]] = {}
    for record in records:
        categories.setdefault(record["category"], []).append(record)

    summary = {
        "total_images": len(records),
        "categories": {
            name: bucket_stats(items) for name, items in sorted(categories.items())
        },
        "overall": bucket_stats(records),
    }

    resolutions: dict[str, list[dict]] = {}
    for record in records:
        edge = record.get("long_edge", max(record["width"], record["height"]))
        for label, low, high in RESOLUTION_BUCKETS:
            if low <= edge <= high:
                resolutions.setdefault(label, []).append(record)
                break
    summary["resolutions"] = {
        label: bucket_stats(items) for label, items in resolutions.items()
    }
    return summary


def markdown(records: list[dict], summary: dict, context: dict) -> str:
    lines = [
        "# Corpus measurements",
        "",
        f"Generated: {context['generated_at']}",
        f"VPNG: `{context['vpng']}`",
        f"Pillow: {context['pillow']}",
        "",
        "Ratios are encoded size divided by canonical pixel bytes on the same pixels.",
        "",
        "| id | category | dimensions | format | canonical | vpng raw | vpng palette | vpng rle | vpng predictor | vpng auto | PNG | WebP |",
        "|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]

    def cell(node: dict) -> str:
        if "bytes" not in node:
            return "-"
        return f"{node['bytes']:,} ({node['ratio']:.3f}x)"

    for record in records:
        lines.append(
            "| {id} | {category} | {width}x{height} | {pixel_format} | {canonical:,} | {raw} | {palette} | {rle} | {predictor} | {auto} | {png} | {webp} |".format(
                id=record["id"],
                category=record["category"],
                width=record["width"],
                height=record["height"],
                pixel_format=record["pixel_format"],
                canonical=record["canonical_bytes"],
                raw=cell(record["vpng"].get("raw", {})),
                palette=cell(record["vpng"].get("palette", {})),
                rle=cell(record["vpng"].get("rle", {})),
                predictor=cell(record["vpng"].get("predictor", {})),
                auto=cell(record["vpng"].get("auto", {})),
                png=cell(record["baselines"].get("png", {})),
                webp=cell(record["baselines"].get("webp", {})),
            )
        )

    lines += [
        "",
        "## Category medians",
        "",
        "| category | images | auto | raw | palette | rle | predictor | PNG | WebP | auto <= best forced | under 10 MB |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for category, row in summary["categories"].items():
        def fmt(value):
            return "-" if value is None else f"{value:.3f}x"

        def med(tool):
            node = row.get(tool, {})
            if not node:
                return "-"
            stats = node.get("ratio", {})
            value = stats.get("median")
            return "-" if value is None else f"{value:.3f}x"

        lines.append(
            f"| {category} | {row['count']} | {med('auto')} | {med('raw')} | "
            f"{med('palette')} | {med('rle')} | {med('predictor')} | {med('png')} | {med('webp')} | "
            f"{row['auto_no_worse_than_best']}/{row['auto_comparable']} | "
            f"{row['under_10mb']['count']}/{row['under_10mb']['total']} |"
        )
    lines += [
        "",
        "FLIF was not compared: no encoder is installed in this environment.",
        "Large images above the baseline pixel cap were encoded by VPNG only.",
        "",
    ]
    return "\n".join(lines)


def self_test() -> int:
    """Pin the percentile rules the report quotes.

    The Rust benchmark harness has the same rules in unit tests. These are
    the Python counterpart: the report's median, p95 and stdev figures are
    only meaningful if the rules are pinned, and the rules are the same in
    both places.
    """
    failures: list[str] = []

    def expect(name: str, actual, wanted) -> None:
        if actual != wanted:
            failures.append(f"{name}: {actual!r} != {wanted!r}")

    expect("median of one", median([7.0]), 7.0)
    expect("median of three", median([3.0, 1.0, 2.0]), 2.0)
    expect("median of an even count", median([1.0, 2.0, 3.0, 4.0]), 2.5)
    expect("median of an empty sample", median([]), None)

    # Nearest-rank p95: with ten samples it is the largest of the ten.
    ten = [float(value) for value in range(1, 11)]
    expect("p95 of ten is the maximum", p95(ten), 10.0)
    twenty = [float(value) for value in range(1, 21)]
    expect("p95 of twenty", p95(twenty), 19.0)
    hundred = [float(value) for value in range(1, 101)]
    expect("p95 of a hundred", p95(hundred), 95.0)
    expect("p95 of an empty sample", p95([]), None)

    # The sample standard deviation, with n-1 in the denominator.
    expect(
        "sample stdev",
        round(stdev([2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]), 5),
        2.13809,
    )
    expect("stdev of one sample", stdev([1.0]), None)

    if failures:
        print(f"{len(failures)} failure(s):")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("the percentile rules are pinned")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(REPO / "corpus-data"))
    parser.add_argument("--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json"))
    parser.add_argument("--vpng", default=str(REPO / "target" / "release" / "vpng.exe"))
    parser.add_argument("--results", default=str(REPO / "benchmarks" / "results"))
    parser.add_argument("--output", default=None, help="also write the JSON report here")
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--category", action="append", default=[])
    parser.add_argument("--codecs", default=",".join(CODECS))
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--workers", type=int, default=8, help="encoder worker threads")
    parser.add_argument("--jobs", type=int, default=1, help="images measured at the same time")
    parser.add_argument("--max-baseline-pixels", type=int, default=32_000_000)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="check the percentile rules and exit",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

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

    codecs = [codec.strip() for codec in args.codecs.split(",") if codec.strip()]
    for codec in codecs:
        if codec not in CODECS:
            raise SystemExit(f"unknown codec `{codec}`; known codecs: {', '.join(CODECS)}")

    results_dir = Path(args.results)
    results_dir.mkdir(parents=True, exist_ok=True)
    work_root = results_dir / "corpus-work"

    started = time.perf_counter()
    records: list[dict] = []

    def measure(entry: dict) -> dict | None:
        work = work_root / entry["id"]
        record = measure_entry(
            entry, root, work, vpng, args.max_baseline_pixels, args.tile_size, args.workers, codecs
        )
        if record is None:
            return None
        auto = record["vpng"].get("auto", {})
        png = record["baselines"].get("png", {})
        webp = record["baselines"].get("webp", {})
        print(
            "   canonical {:,}  auto {}  png {}  webp {}  auto<=best {}".format(
                record["canonical_bytes"],
                f"{auto['bytes']:,} ({auto['ratio']:.3f}x)" if "bytes" in auto else "failed",
                f"{png['bytes']:,}" if "bytes" in png else "-",
                f"{webp['bytes']:,}" if "bytes" in webp else "-",
                record.get("auto_no_worse_than_best"),
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
    summary = summarise(records, codecs)
    context = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "vpng": str(vpng),
        "pillow": getattr(Image, "__version__", "unknown"),
        "python": sys.version.split()[0],
        "parameters": {
            "tile_size": args.tile_size,
            "workers": args.workers,
            "codecs": codecs,
            "jobs": args.jobs,
            "max_baseline_pixels": args.max_baseline_pixels,
            "size_ceiling_bytes": SIZE_CEILING_BYTES,
        },
        "elapsed_seconds": round(time.perf_counter() - started, 3),
    }
    report = {"context": context, "summary": summary, "images": records}
    json_path = results_dir / f"corpus_{stamp}.json"
    md_path = results_dir / f"corpus_{stamp}.md"
    json_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    md_path.write_text(markdown(records, summary, context), encoding="utf-8")
    if args.output:
        output = Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print()
    for category, row in summary["categories"].items():
        def fmt(value):
            return "-" if value is None else f"{value:.3f}x"

        def med(tool):
            node = row.get(tool, {})
            if not node:
                return None
            return node.get("ratio", {}).get("median")

        print(
            f"{category:<12} n={row['count']:<4} auto {fmt(med('auto'))}  "
            f"png {fmt(med('png'))}  webp {fmt(med('webp'))}  "
            f"under 10MB {row['under_10mb']['count']}/{row['under_10mb']['total']}"
        )
    print(f"report: {json_path}")
    print(f"summary: {md_path}")
    if args.output:
        print(f"stable output: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
