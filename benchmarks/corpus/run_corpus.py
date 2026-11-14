#!/usr/bin/env python3
"""Run the seed corpus through VPNG and the available lossless baselines.

Reads `benchmarks/corpus_manifest.json`, canonicalises each image to raw pixels
(Pillow), encodes it with every implemented VPNG codec, and compares sizes with
PNG and WebP-lossless on the *same* canonical pixels. Results are written to the
git-ignored `benchmarks/results/` directory as JSON and Markdown.

Run from the repository root:
    python benchmarks/corpus/run_corpus.py
    python benchmarks/corpus/run_corpus.py --only user-photo-15360x8640
"""

import argparse
import json
import statistics
import struct
import subprocess
import sys
import time
from pathlib import Path

from PIL import Image, features

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "benchmarks" / "stress"))
from canonicalize import convert_image  # noqa: E402

CODECS = ["raw", "palette", "rle", "auto"]
PIL_MODES = {"gray8": "L", "graya8": "LA", "rgb8": "RGB", "rgba8": "RGBA"}


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


def measure_entry(entry: dict, root: Path, work: Path, vpng: Path, max_baseline_pixels: int) -> dict | None:
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
        "--tile-size", "256",
    ]

    record = {
        "id": entry["id"],
        "category": entry["category"],
        "width": width,
        "height": height,
        "pixel_format": fmt,
        "canonical_bytes": meta["bytes"],
        "canonical_sha256": meta["sha256"],
        "sha256_matches_manifest": sha_matches,
        "vpng": {},
        "baselines": {},
    }

    for codec in CODECS:
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
        with raw.open("rb") as fh:
            pixels = fh.read()
        image = Image.frombytes(PIL_MODES[fmt], (width, height), pixels)
        png = work / "baseline.png"
        started = time.perf_counter()
        image.save(png, format="PNG", compress_level=6)
        record["baselines"]["png"] = {
            "bytes": png.stat().st_size,
            "ratio": round(png.stat().st_size / meta["bytes"], 5),
            "ms": round((time.perf_counter() - started) * 1000),
            "settings": "Pillow PNG, compress_level=6",
        }
        png.unlink(missing_ok=True)

        if features.check("webp"):
            webp = work / "baseline.webp"
            started = time.perf_counter()
            try:
                image.save(webp, format="WEBP", lossless=True, quality=100, method=4)
                record["baselines"]["webp"] = {
                    "bytes": webp.stat().st_size,
                    "ratio": round(webp.stat().st_size / meta["bytes"], 5),
                    "ms": round((time.perf_counter() - started) * 1000),
                    "settings": "Pillow WebP lossless, quality=100, method=4",
                }
            except Exception as exc:  # noqa: BLE001
                record["baselines"]["webp"] = {"error": str(exc)}
            webp.unlink(missing_ok=True)
        else:
            record["baselines"]["webp"] = {"error": "Pillow build has no WebP support"}
    else:
        record["baselines"]["skipped"] = (
            f"{width * height} pixels exceeds the {max_baseline_pixels}-pixel baseline cap"
        )

    record["baselines"]["jxl"] = {"error": "no JPEG XL encoder installed"}
    record["baselines"]["flif"] = {"error": "no FLIF encoder installed"}

    forced = [record["vpng"][codec]["bytes"] for codec in ("raw", "palette", "rle") if "bytes" in record["vpng"][codec]]
    if forced and "bytes" in record["vpng"]["auto"]:
        best = min(forced)
        auto = record["vpng"]["auto"]["bytes"]
        record["best_forced_bytes"] = best
        # Auto selection may mix codecs per tile, so it can beat every single-codec
        # run; it must never be larger than the best forced run.
        record["auto_no_worse_than_best"] = auto <= best
        record["auto_saved_bytes_vs_best"] = best - auto

    raw.unlink(missing_ok=True)
    work.rmdir()
    return record


def summarise(records: list[dict]) -> dict:
    categories = {}
    for record in records:
        bucket = categories.setdefault(record["category"], [])
        bucket.append(record)

    summary = {}
    for category, items in sorted(categories.items()):
        def median_ratio(path: list[str]) -> float | None:
            values = []
            for item in items:
                node = item
                for key in path:
                    node = node.get(key, {}) if isinstance(node, dict) else {}
                if isinstance(node, dict) and "ratio" in node:
                    values.append(node["ratio"])
            return round(statistics.median(values), 5) if values else None

        summary[category] = {
            "count": len(items),
            "median_auto": median_ratio(["vpng", "auto"]),
            "median_raw": median_ratio(["vpng", "raw"]),
            "median_palette": median_ratio(["vpng", "palette"]),
            "median_rle": median_ratio(["vpng", "rle"]),
            "median_png": median_ratio(["baselines", "png"]),
            "median_webp": median_ratio(["baselines", "webp"]),
            "auto_no_worse_than_best": sum(1 for item in items if item.get("auto_no_worse_than_best")),
        }
    return summary


def markdown(records: list[dict], summary: dict, context: dict) -> str:
    lines = [
        "# Seed-corpus measurements",
        "",
        f"Generated: {context['generated_at']}",
        f"VPNG: `{context['vpng']}`",
        f"Pillow: {context['pillow']}",
        "",
        "Ratios are encoded size divided by canonical pixel bytes on the same pixels.",
        "The seed subset is not the 750-image target; claims apply to these images only.",
        "",
        "| id | category | dimensions | format | canonical | vpng raw | vpng palette | vpng rle | vpng auto | PNG | WebP |",
        "|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ]

    def cell(node: dict) -> str:
        if "bytes" not in node:
            return "-"
        return f"{node['bytes']:,} ({node['ratio']:.3f}x)"

    for record in records:
        lines.append(
            "| {id} | {category} | {width}x{height} | {pixel_format} | {canonical:,} | {raw} | {palette} | {rle} | {auto} | {png} | {webp} |".format(
                id=record["id"],
                category=record["category"],
                width=record["width"],
                height=record["height"],
                pixel_format=record["pixel_format"],
                canonical=record["canonical_bytes"],
                raw=cell(record["vpng"].get("raw", {})),
                palette=cell(record["vpng"].get("palette", {})),
                rle=cell(record["vpng"].get("rle", {})),
                auto=cell(record["vpng"].get("auto", {})),
                png=cell(record["baselines"].get("png", {})),
                webp=cell(record["baselines"].get("webp", {})),
            )
        )

    lines += [
        "",
        "## Category medians",
        "",
        "| category | images | auto | raw | palette | rle | PNG | WebP | auto <= best forced |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for category, row in summary.items():
        def fmt(value):
            return "-" if value is None else f"{value:.3f}x"

        lines.append(
            f"| {category} | {row['count']} | {fmt(row['median_auto'])} | {fmt(row['median_raw'])} | "
            f"{fmt(row['median_palette'])} | {fmt(row['median_rle'])} | {fmt(row['median_png'])} | "
            f"{fmt(row['median_webp'])} | {row['auto_no_worse_than_best']}/{row['count']} |"
        )
    lines += [
        "",
        "JXL and FLIF were not compared: no encoder is installed in this environment.",
        "Large images above the baseline pixel cap were encoded by VPNG only.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(REPO / "corpus-data"))
    parser.add_argument("--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json"))
    parser.add_argument("--vpng", default=str(REPO / "target" / "release" / "vpng.exe"))
    parser.add_argument("--results", default=str(REPO / "benchmarks" / "results"))
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--category", action="append", default=[])
    parser.add_argument("--max-baseline-pixels", type=int, default=32_000_000)
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
    work_root = results_dir / "corpus-work"

    records = []
    for entry in entries:
        print(f"== {entry['id']} ({entry['category']})")
        work = work_root / entry["id"]
        record = measure_entry(entry, root, work, vpng, args.max_baseline_pixels)
        if record is None:
            continue
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
        records.append(record)

    stamp = time.strftime("%Y%m%d_%H%M%S")
    summary = summarise(records)
    context = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "vpng": str(vpng),
        "pillow": getattr(Image, "__version__", "unknown"),
    }
    report = {"context": context, "summary": summary, "images": records}
    json_path = results_dir / f"corpus_{stamp}.json"
    md_path = results_dir / f"corpus_{stamp}.md"
    json_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    md_path.write_text(markdown(records, summary, context), encoding="utf-8")

    print()
    for category, row in summary.items():
        def fmt(value):
            return "-" if value is None else f"{value:.3f}x"

        print(
            f"{category:<18} n={row['count']:<3} auto {fmt(row['median_auto'])}  "
            f"raw {fmt(row['median_raw'])}  png {fmt(row['median_png'])}  webp {fmt(row['median_webp'])}  "
            f"auto<=best {row['auto_no_worse_than_best']}/{row['count']}"
        )
    print(f"report: {json_path}")
    print(f"summary: {md_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
