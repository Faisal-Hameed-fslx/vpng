#!/usr/bin/env python3
"""Render the P5.2 report's tables from the raw benchmark data.

The report quotes figures; this script produces those tables directly from
`benchmarks/p5_bench_results.json`, so a table can never disagree with the
data it came from. `benchmarks/verify_reports.py` then checks that the
figures the report actually contains are the ones in the raw file.

Run from the repository root:

    python benchmarks/render_bench_tables.py
"""

import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
RAW = REPO / "benchmarks/p5_bench_results.json"


def ms(value: float) -> str:
    """A latency in milliseconds, with no thousands separator.

    The verifier looks for the raw figure in this exact form, so the tables
    and the report agree by construction.
    """
    return f"{value:.2f}"


def bytes_(value: int) -> str:
    """A byte count with thousands separators."""
    return f"{value:,}"


def ratio(value: float) -> str:
    return f"{value:.3f}"


def main() -> int:
    if not RAW.exists():
        print(f"{RAW} does not exist; run the benchmark first", file=sys.stderr)
        return 1

    raw = json.loads(RAW.read_text(encoding="utf-8"))
    report = raw["report"]
    scenarios = report["scenarios"]
    tile_sizes = report["parameters"]["tile_sizes"]
    workers = report["parameters"]["worker_counts"]
    runs = report["parameters"]["runs"]

    encode = {(row["tile_size"], row["workers"]): row for row in scenarios["encode_matrix"]}
    viewport = {
        (row["tile_size"], row["workers"]): row
        for row in scenarios["first_viewport_matrix"]
    }
    pan = {(row["tile_size"], row["workers"]): row for row in scenarios["pan_matrix"]}
    baseline = {row["tile_size"]: row for row in scenarios["sequential_baseline"]}
    compression = {row["tile_size"]: row for row in scenarios["compression"]}

    # ---- the encode matrix --------------------------------------------------
    print("### 3.1 The encode matrix\n")
    print("The time to encode the whole 16K photograph, in milliseconds.\n")
    print("| Tile size | Workers | Median | p95 | Mean | Stdev | Min | Max | Speedup vs 1 worker |")
    print("|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for tile in tile_sizes:
        for count in workers:
            row = encode[(tile, count)]
            stats = row["stats"]
            base = encode[(tile, 1)]["stats"]["median_ms"]
            speedup = base / stats["median_ms"] if stats["median_ms"] else 0.0
            print(
                f"| {tile} | {count} | {ms(stats['median_ms'])} | {ms(stats['p95_ms'])} "
                f"| {ms(stats['mean_ms'])} | {ms(stats['stdev_ms'])} | {ms(stats['min_ms'])} "
                f"| {ms(stats['max_ms'])} | {speedup:.2f}x |"
            )
    print()

    # ---- the compression matrix ---------------------------------------------
    print("### 3.2 What the tile size costs and buys\n")
    print("| Tile size | Tiles | File size | Payload bytes | Ratio | SHA-256 |")
    print("|---:|---:|---:|---:|---:|---|")
    for tile in tile_sizes:
        row = compression[tile]
        print(
            f"| {tile} | {bytes_(row['tiles'])} | {bytes_(row['file_size_bytes'])} "
            f"| {bytes_(row['payload_bytes'])} | {ratio(row['ratio'])} | `{row['sha256']}` |"
        )
    print()

    # ---- the first viewport --------------------------------------------------
    print("### 3.3 The first-viewport latency matrix\n")
    print("The time from opening the file to the first pixels on screen, in\n"
          "milliseconds, against the sequential baseline measured in the same run.\n")
    print("| Tile size | Workers | Visible tiles | Open (median) | Decode (median) | Time to pixels (median) | p95 | Sequential baseline | Speedup |")
    print("|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for tile in tile_sizes:
        base = baseline[tile]["stats"]["median_ms"]
        for count in workers:
            row = viewport[(tile, count)]
            stats = row["time_to_pixels_stats"]
            speedup = base / stats["median_ms"] if stats["median_ms"] else 0.0
            print(
                f"| {tile} | {count} | {row['visible_tiles']} "
                f"| {ms(row['open_stats']['median_ms'])} | {ms(row['decode_stats']['median_ms'])} "
                f"| {ms(stats['median_ms'])} | {ms(stats['p95_ms'])} "
                f"| {ms(base)} | {speedup:.2f}x |"
            )
    print()

    # ---- the pan -------------------------------------------------------------
    print("### 3.4 The pan latency\n")
    print("Twelve pans to the right, each 512 px, per run. The per-pan figures\n"
          "cover every pan of every run.\n")
    print("| Tile size | Workers | Pans | Median pan | p95 pan | Slowest pan | Median sweep |")
    print("|---:|---:|---:|---:|---:|---:|---:|")
    for tile in tile_sizes:
        for count in workers:
            row = pan[(tile, count)]
            stats = row["pan_stats"]
            print(
                f"| {tile} | {count} | {row['pans_per_run']} | {ms(stats['median_ms'])} "
                f"| {ms(stats['p95_ms'])} | {ms(stats['max_ms'])} "
                f"| {ms(row['sweep_total_stats']['median_ms'])} |"
            )
    print()

    # ---- the run-to-run spread ------------------------------------------------
    print("### 3.5 How much a single run can be trusted\n")
    print("The spread of the ten samples, as a percentage of the median.\n")
    print("| Tile size | Workers | Spread (max - min) | As % of median | Stdev as % of median |")
    print("|---:|---:|---:|---:|---:|")
    for tile in tile_sizes:
        for count in workers:
            stats = encode[(tile, count)]["stats"]
            spread = stats["max_ms"] - stats["min_ms"]
            print(
                f"| {tile} | {count} | {ms(spread)} ms | {100.0 * spread / stats['median_ms']:.1f}% "
                f"| {100.0 * stats['stdev_ms'] / stats['median_ms']:.1f}% |"
            )
    print()
    print(f"Every case ran {runs} times. The digests of every file written during the\n"
          f"run: {', '.join(sorted({row['sha256'] for row in scenarios['encode_matrix']}))}.")
    print()
    print(f"Process peak working set: {bytes_(raw['process']['peak_working_set_bytes'])} bytes; "
          f"peak heap: {bytes_(raw['process']['peak_heap_bytes'])} bytes; "
          f"wall clock: {ms(raw['process']['elapsed_ms'])} ms.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
