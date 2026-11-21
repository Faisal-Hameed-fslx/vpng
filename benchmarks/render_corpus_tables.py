#!/usr/bin/env python3
"""Render the corpus report's tables from the raw measurement data.

The report quotes figures; this script produces those tables directly from
`benchmarks/p5_corpus_results.json`, so a table can never disagree with the
data it came from. `benchmarks/verify_reports.py` then checks that the
figures the report actually contains are the ones in the raw file.

Run from the repository root:

    python benchmarks/render_corpus_tables.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
RAW = REPO / "benchmarks" / "p5_corpus_results.json"
MANIFEST = REPO / "benchmarks" / "corpus_manifest.json"


def bytes_(value) -> str:
    """A byte count with thousands separators."""
    if value is None:
        return "-"
    return f"{int(value):,}"


def ratio(value) -> str:
    """A ratio with three decimals."""
    if value is None:
        return "-"
    return f"{value:.3f}"


def percent(value) -> str:
    """A share of images, as a percentage with one decimal."""
    if value is None:
        return "-"
    return f"{100.0 * value:.1f}%"


def row(values: dict, key: str, stat: str = "median"):
    """The median (or another statistic) of one codec's ratio distribution."""
    node = values.get(key)
    if not node:
        return None
    return node.get("ratio", {}).get(stat)


def main() -> int:
    if not RAW.exists():
        print(f"{RAW} does not exist; run the corpus benchmark first", file=sys.stderr)
        return 1

    raw = json.loads(RAW.read_text(encoding="utf-8"))
    summary = raw["summary"]
    categories = summary["categories"]
    overall = summary["overall"]
    images = raw["images"]
    parameters = raw["context"]["parameters"]

    print("### 3.1 The corpus\n")
    print("| Category | Images | Real | Synthetic | Median pixels | Median long edge |")
    print("|---|---:|---:|---:|---:|---:|")
    for name, row_data in categories.items():
        bucket = [image for image in images if image["category"] == name]
        pixels = sorted(image["canonical_bytes"] for image in bucket)
        edges = sorted(image["long_edge"] for image in bucket)
        middle = len(pixels) // 2
        median_pixels = pixels[middle] if len(pixels) % 2 else (pixels[middle - 1] + pixels[middle]) // 2
        print(
            f"| {name} | {row_data['count']} | {row_data['real']} | {row_data['synthetic']} "
            f"| {bytes_(median_pixels)} | {bytes_(edges[middle] if len(edges) % 2 else (edges[middle - 1] + edges[middle]) // 2)} |"
        )
    print(
        f"| **total** | **{overall['count']}** | **{overall['real']}** | **{overall['synthetic']}** "
        f"| | |"
    )
    print()

    print("### 3.2 What each codec does, per category\n")
    print("Ratios are encoded size divided by canonical pixel bytes, on the same pixels.\n")
    print("| Category | Images | auto | raw | palette | rle | predictor | PNG | WebP | JPEG XL |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for name, row_data in categories.items():
        print(
            f"| {name} | {row_data['count']} | {ratio(row(row_data, 'auto'))} "
            f"| {ratio(row(row_data, 'raw'))} | {ratio(row(row_data, 'palette'))} "
            f"| {ratio(row(row_data, 'rle'))} | {ratio(row(row_data, 'predictor'))} "
            f"| {ratio(row(row_data, 'png'))} | {ratio(row(row_data, 'webp'))} "
            f"| {ratio(row(row_data, 'jxl'))} |"
        )
    print(
        f"| **all** | {overall['count']} | {ratio(row(overall, 'auto'))} "
        f"| {ratio(row(overall, 'raw'))} | {ratio(row(overall, 'palette'))} "
        f"| {ratio(row(overall, 'rle'))} | {ratio(row(overall, 'predictor'))} "
        f"| {ratio(row(overall, 'png'))} | {ratio(row(overall, 'webp'))} "
        f"| {ratio(row(overall, 'jxl'))} |"
    )
    print()

    print("### 3.3 The size distribution\n")
    print("The auto codec's output size, in bytes. The p95 is the **nearest-rank**\n"
          "percentile: the smallest sample at or above the 95th percentile.\n")
    print("| Category | Images | Median | p95 | Largest | Median ratio | p95 ratio |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for name, row_data in categories.items():
        auto = row_data["auto"]
        print(
            f"| {name} | {row_data['count']} | {bytes_(auto['bytes']['median'])} "
            f"| {bytes_(auto['bytes']['p95'])} | {bytes_(auto['bytes']['max'])} "
            f"| {ratio(auto['ratio']['median'])} | {ratio(auto['ratio']['p95'])} |"
        )
    print(
        f"| **all** | {overall['count']} | {bytes_(overall['auto']['bytes']['median'])} "
        f"| {bytes_(overall['auto']['bytes']['p95'])} | {bytes_(overall['auto']['bytes']['max'])} "
        f"| {ratio(overall['auto']['ratio']['median'])} | {ratio(overall['auto']['ratio']['p95'])} |"
    )
    print()

    print("### 3.4 The 10 MB target\n")
    print(f"The ceiling is {bytes_(parameters['size_ceiling_bytes'])} bytes "
          "(decimal, ADR-0006).\n")
    print("| Category | Images | Under the ceiling | Share | Median | p95 |")
    print("|---|---:|---:|---:|---:|---:|")
    for name, row_data in categories.items():
        under = row_data["under_10mb"]
        auto = row_data["auto"]
        print(
            f"| {name} | {row_data['count']} | {under['count']} | {percent(under['rate'])} "
            f"| {bytes_(auto['bytes']['median'])} | {bytes_(auto['bytes']['p95'])} |"
        )
    overall_under = overall["under_10mb"]
    print(
        f"| **all** | {overall['count']} | {overall_under['count']} | {percent(overall_under['rate'])} "
        f"| {bytes_(overall['auto']['bytes']['median'])} | {bytes_(overall['auto']['bytes']['p95'])} |"
    )
    print()

    print("### 3.5 The distribution by resolution\n")
    print("| Long edge | Images | auto (median) | PNG (median) | WebP (median) | Under 10 MB |")
    print("|---|---:|---:|---:|---:|---:|")
    for name in ("<=512", "513-1024", "1025-2048", "2049-4096", "4097-8192", ">8192"):
        row_data = summary["resolutions"].get(name)
        if not row_data:
            continue
        under = row_data["under_10mb"]
        print(
            f"| {name} | {row_data['count']} | {ratio(row(row_data, 'auto'))} "
            f"| {ratio(row(row_data, 'png'))} | {ratio(row(row_data, 'webp'))} "
            f"| {under['count']}/{under['total']} |"
        )
    print()

    print("### 3.6 The worst cases\n")
    print("| Image | Category | Pixels | Canonical | auto | Ratio | PNG |")
    print("|---|---|---:|---:|---:|---:|---:|")
    ranked = sorted(
        images,
        key=lambda image: image["vpng"].get("auto", {}).get("bytes", 0),
        reverse=True,
    )
    for image in ranked[:10]:
        auto = image["vpng"].get("auto", {})
        png = image["baselines"].get("png", {})
        print(
            f"| {image['id']} | {image['category']} | {bytes_(image['width'])}x{bytes_(image['height'])} "
            f"| {bytes_(image['canonical_bytes'])} | {bytes_(auto.get('bytes'))} "
            f"| {ratio(auto.get('ratio'))} | {bytes_(png.get('bytes'))} |"
        )
    print()

    total_canonical = overall["total_canonical_bytes"]
    total_auto = overall["total_auto_bytes"]
    print(f"The corpus holds {bytes_(total_canonical)} canonical bytes; the auto codec "
          f"wrote {bytes_(total_auto)} bytes of files for them, a ratio of "
          f"{ratio(total_auto / total_canonical) if total_canonical else '-'}.")
    print()

    # ---- the provenance ------------------------------------------------------
    if MANIFEST.exists():
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        counts: dict[str, int] = {}
        for entry in manifest["images"]:
            key = entry.get("source_id", "seed")
            counts[key] = counts.get(key, 0) + 1
        print("### 3.7 Where the images came from\n")
        print("| Source | Repository | Commit | Category | Images |")
        print("|---|---|---|---|---:|")
        for source in manifest.get("sources", []):
            print(
                f"| {source['id']} | `{source['repo']}` | `{source['commit'][:12]}` "
                f"| {source['category']} | {counts.get(source['id'], 0)} |"
            )
        seed_count = counts.get("seed", 0)
        print(f"| seed (local) | the repository's own seed corpus | - | mixed | {seed_count} |")
        print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
