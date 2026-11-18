#!/usr/bin/env python3
"""Run the P3.5 compression study over the seed corpus.

The study measures every stable and experimental codec on the **same canonical
pixels** and the **same tile layout**, and compares the result with the
available lossless baselines (PNG, WebP lossless, JPEG XL where installed).

Two artefacts are produced:

* ``benchmarks/p3_5_raw_results.json`` — the raw data: per image, per tile,
  per category, per resolution, aggregate and worst case.
* ``benchmarks/p3_5_comparison.csv`` — the same numbers as a flat table with
  a ``scope`` column, so one file can be pivoted in a spreadsheet.

Nothing here writes a VPNG file with an experimental codec id. The research
codecs are measured by the ``vpng-research`` binary, which only ever produces
payloads in memory; the production encoder cannot reference them.

Run from the repository root:

    python benchmarks/corpus/run_research.py
    python benchmarks/corpus/run_research.py --only user-photo-15360x8640
"""

import argparse
import csv
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "benchmarks" / "stress"))
sys.path.insert(0, str(HERE))
from baselines import measure_baselines  # noqa: E402
from canonicalize import convert_image  # noqa: E402

#: The manifest's resolution ladder. An image is filed under the smallest
#: ladder entry that is at least as large as its longest edge.
RESOLUTION_LADDER = [512, 1024, 2048, 4096, 8192, 16384]

#: The candidates the research binary reports, in its fixed order.
CANDIDATE_ORDER = [
    "raw",
    "palette",
    "rle",
    "predictor",
    "carc-a",
    "carc-b",
    "carc-c",
    "carc-d",
    "lz77",
    "pred-lz77",
    "lz77-entropy",
    "pred-lz77-entropy",
]


def resolution_class(width: int, height: int) -> int:
    """The ladder entry an image is filed under."""
    longest = max(width, height)
    for rung in RESOLUTION_LADDER:
        if longest <= rung:
            return rung
    return RESOLUTION_LADDER[-1]


def run_research(binary: Path, raw: Path, width: int, height: int, fmt: str, tile: int, out: Path) -> dict:
    """Run the research harness over one canonical raw file."""
    started = time.perf_counter()
    proc = subprocess.run(
        [
            str(binary),
            "--input", str(raw),
            "--width", str(width),
            "--height", str(height),
            "--format", fmt,
            "--tile-size", str(tile),
            "--output", str(out),
        ],
        capture_output=True,
        text=True,
        timeout=7200,
    )
    elapsed = round((time.perf_counter() - started) * 1000)
    if proc.returncode != 0:
        raise RuntimeError(
            f"vpng-research failed for {raw.name} (exit {proc.returncode}): "
            f"{(proc.stdout + proc.stderr).strip()[:400]}"
        )
    document = json.loads(out.read_text(encoding="utf-8"))
    document["wall_ms"] = elapsed
    return document


def load_or_run_research(
    binary: Path,
    raw: Path,
    width: int,
    height: int,
    fmt: str,
    tile: int,
    out: Path,
    cached: Path | None,
    reuse: bool,
) -> dict:
    """Run the harness, or reuse a previously saved raw measurement."""
    if reuse and cached is not None and cached.exists():
        document = json.loads(cached.read_text(encoding="utf-8"))
        print("   reusing the saved measurement", flush=True)
        return document
    return run_research(binary, raw, width, height, fmt, tile, out)


def measure_entry(entry: dict, root: Path, work: Path, binary: Path, tile: int, max_baseline_pixels: int, raw_out: Path, reuse: bool) -> dict | None:
    """Measure one corpus image."""
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
    research_json = work / "research.json"
    print(f"   measuring {width}x{height} {fmt} ...", flush=True)
    raw_out.mkdir(parents=True, exist_ok=True)
    cached = raw_out / f"{entry['id']}.json"
    research = load_or_run_research(
        binary, raw, width, height, fmt, tile, research_json, cached, reuse
    )

    # The full per-tile record, including the per-tile timings and the
    # per-tile heap peaks, is kept verbatim in the git-ignored results
    # directory. The tracked deliverable carries the size and exactness of
    # every tile, which is what the comparison needs.
    if not (reuse and cached.exists()):
        cached.write_text(json.dumps(research, indent=1) + "\n", encoding="utf-8")

    baselines = measure_baselines(
        raw, width, height, fmt, meta["bytes"], work, max_baseline_pixels
    )

    record = {
        "id": entry["id"],
        "category": entry["category"],
        "width": width,
        "height": height,
        "resolution_class": resolution_class(width, height),
        "pixel_format": fmt,
        "canonical_bytes": meta["bytes"],
        "canonical_sha256": meta["sha256"],
        "sha256_matches_manifest": sha_matches,
        "tile_width": tile,
        "tile_height": tile,
        "tile_count": research["input"]["tile_count"],
        "research_wall_ms": research["wall_ms"],
        "candidates": research["per_image"],
        "selection": research["selection"],
        "tiles": compact_tiles(research["tiles"]),
        "baselines": baselines,
        "process": research["process"],
    }

    raw.unlink(missing_ok=True)
    research_json.unlink(missing_ok=True)
    return record


def compact_tiles(tiles: list[dict]) -> list[dict]:
    """The per-tile record, reduced to the columns the comparison needs."""
    out = []
    for tile in tiles:
        results = []
        for result in tile["results"]:
            results.append(
                {
                    "name": result["name"],
                    "status": result["status"],
                    "payload_bytes": result["payload_bytes"],
                    "ratio": result["ratio"],
                    "exact": result["exact"],
                }
            )
        out.append(
            {
                "index": tile["index"],
                "x": tile["x"],
                "y": tile["y"],
                "width": tile["width"],
                "height": tile["height"],
                "canonical_bytes": tile["canonical_bytes"],
                "results": results,
            }
        )
    return out


def container_framing(tile_count: int) -> int:
    """The container bytes a file of `tile_count` tiles carries.

    A 64-byte header, a 40-byte directory entry per tile, and a 4-byte CRC
    trailer over the directory.
    """
    return 64 + tile_count * (40 + 4)


def file_equivalent(record: dict, node: dict) -> dict:
    """The whole-file size a candidate would occupy in a real VPNG file.

    A codec that cannot represent every tile — the palette codec meeting a
    tile with more than 256 colours, for instance — cannot produce a file on
    its own. The honest comparison is the size of a file in which the codec
    handles every tile it can and the raw codec handles the rest. That is
    what this computes, and the raw fallback is reported separately so the
    partial coverage is never hidden.
    """
    payload = node.get("payload_bytes", 0)
    applicable_canonical = node.get("canonical_bytes", 0)
    raw_fallback = max(record["canonical_bytes"] - applicable_canonical, 0)
    framing = container_framing(record["tile_count"])
    file_bytes = payload + raw_fallback + framing
    canonical = record["canonical_bytes"]
    ratio = file_bytes / canonical if canonical else None
    bpp = file_bytes * 8 / canonical if canonical else None
    return {
        "file_bytes": file_bytes,
        "file_ratio": round(ratio, 6) if ratio is not None else None,
        "file_bpp": round(bpp, 6) if bpp is not None else None,
        "raw_fallback_bytes": raw_fallback,
        "container_framing_bytes": framing,
        "covers_all_tiles": applicable_canonical == canonical,
    }


def candidate_row(record: dict, node: dict) -> dict:
    """The comparison columns of one candidate on one image."""
    payload = node.get("payload_bytes", 0)
    canonical = record["canonical_bytes"]
    equivalent = file_equivalent(record, node)
    return {
        "id": record["id"],
        "category": record["category"],
        "width": record["width"],
        "height": record["height"],
        "resolution_class": record["resolution_class"],
        "pixel_format": record["pixel_format"],
        "canonical_bytes": canonical,
        "codec": node["name"],
        "codec_id": node.get("codec_id", ""),
        "bytes": equivalent["file_bytes"],
        "payload_bytes": payload,
        "ratio": equivalent["file_ratio"],
        "bpp": equivalent["file_bpp"],
        "raw_fallback_bytes": equivalent["raw_fallback_bytes"],
        "container_framing_bytes": equivalent["container_framing_bytes"],
        "covers_all_tiles": equivalent["covers_all_tiles"],
        "encode_ms": node.get("encode_ms"),
        "decode_ms": node.get("decode_ms"),
        "exact": node.get("exact"),
        "framing_bytes": node.get("framing_bytes"),
        "model_state_bytes": node.get("model_state_bytes"),
        "peak_encode_alloc_bytes": node.get("peak_encode_alloc_bytes"),
        "peak_decode_alloc_bytes": node.get("peak_decode_alloc_bytes"),
        "tiles_applicable": node.get("tiles_applicable"),
        "tiles_failed": node.get("tiles_failed"),
        "status": node.get("status", "ok"),
        "source": "vpng-research",
    }


def baseline_row(record: dict, name: str, node: dict) -> dict:
    """The comparison columns of one baseline on one image."""
    return {
        "id": record["id"],
        "category": record["category"],
        "width": record["width"],
        "height": record["height"],
        "resolution_class": record["resolution_class"],
        "pixel_format": record["pixel_format"],
        "canonical_bytes": record["canonical_bytes"],
        "codec": name,
        "codec_id": "",
        "bytes": node.get("bytes"),
        "payload_bytes": node.get("bytes"),
        "ratio": node.get("ratio"),
        "bpp": node.get("bpp"),
        "raw_fallback_bytes": 0,
        "container_framing_bytes": 0,
        "covers_all_tiles": True,
        "encode_ms": node.get("ms"),
        "decode_ms": None,
        "exact": None,
        "framing_bytes": None,
        "model_state_bytes": None,
        "peak_encode_alloc_bytes": None,
        "peak_decode_alloc_bytes": None,
        "tiles_applicable": None,
        "tiles_failed": None,
        "status": "ok" if "bytes" in node else node.get("error", "unavailable"),
        "source": "baseline",
    }


def collect_rows(records: list[dict]) -> list[dict]:
    """Every image-level comparison row."""
    rows = []
    for record in records:
        for node in record["candidates"]:
            rows.append(candidate_row(record, node))
        for name, node in record["baselines"].items():
            if name == "skipped":
                continue
            rows.append(baseline_row(record, name, node))
    return rows


def bucket_aggregate(rows: list[dict], key: str) -> dict:
    """Aggregate comparison rows by one grouping key."""
    buckets: dict = {}
    for row in rows:
        bucket = buckets.setdefault(row[key], {})
        codec = bucket.setdefault(
            row["codec"],
            {
                "bytes": 0,
                "canonical_bytes": 0,
                "images": 0,
                "ratios": [],
                "exact": True,
                "raw_fallback_bytes": 0,
                "covers_all_tiles": True,
            },
        )
        if row["bytes"] is None:
            continue
        codec["bytes"] += row["bytes"]
        codec["canonical_bytes"] += row["canonical_bytes"]
        codec["images"] += 1
        codec["raw_fallback_bytes"] += row.get("raw_fallback_bytes") or 0
        if row.get("covers_all_tiles") is False:
            codec["covers_all_tiles"] = False
        if row["ratio"] is not None:
            codec["ratios"].append(row["ratio"])
        if row["exact"] is False:
            codec["exact"] = False
    for bucket in buckets.values():
        for codec in bucket.values():
            codec["ratio"] = (
                round(codec["bytes"] / codec["canonical_bytes"], 6)
                if codec["canonical_bytes"]
                else None
            )
            codec["bpp"] = (
                round(codec["bytes"] * 8 / codec["canonical_bytes"], 6)
                if codec["canonical_bytes"]
                else None
            )
            codec["median_ratio"] = (
                round(statistics.median(codec["ratios"]), 6) if codec["ratios"] else None
            )
            codec["worst_ratio"] = max(codec["ratios"]) if codec["ratios"] else None
            codec["best_ratio"] = min(codec["ratios"]) if codec["ratios"] else None
            codec.pop("ratios", None)
    return buckets


def worst_cases(records: list[dict]) -> dict:
    """The worst image and the worst tile for every candidate."""
    worst: dict = {}
    for record in records:
        for node in record["candidates"]:
            name = node["name"]
            ratio = node.get("file_ratio", node.get("ratio"))
            if ratio is None:
                continue
            entry = worst.setdefault(name, {"worst_image": None, "worst_tile": None})
            if entry["worst_image"] is None or ratio > entry["worst_image"]["ratio"]:
                entry["worst_image"] = {
                    "id": record["id"],
                    "category": record["category"],
                    "resolution_class": record["resolution_class"],
                    "ratio": ratio,
                    "bytes": node.get("file_bytes", node.get("payload_bytes")),
                }
            # The worst tile is the tile with the largest payload ratio for
            # this candidate. Only tiles the codec could represent count.
            for tile in record["tiles"]:
                for result in tile["results"]:
                    if result["name"] != name or result["status"] != "ok":
                        continue
                    tile_ratio = result["ratio"]
                    if entry["worst_tile"] is None or tile_ratio > entry["worst_tile"]["ratio"]:
                        entry["worst_tile"] = {
                            "id": record["id"],
                            "tile_index": tile["index"],
                            "x": tile["x"],
                            "y": tile["y"],
                            "width": tile["width"],
                            "height": tile["height"],
                            "ratio": tile_ratio,
                            "payload_bytes": result["payload_bytes"],
                        }
    return worst


def write_csv(path: Path, records: list[dict]) -> int:
    """Write the flat comparison table."""
    columns = [
        "scope",
        "group",
        "id",
        "category",
        "width",
        "height",
        "resolution_class",
        "pixel_format",
        "canonical_bytes",
        "codec",
        "codec_id",
        "bytes",
        "payload_bytes",
        "ratio",
        "bpp",
        "raw_fallback_bytes",
        "container_framing_bytes",
        "covers_all_tiles",
        "encode_ms",
        "decode_ms",
        "exact",
        "framing_bytes",
        "model_state_bytes",
        "peak_encode_alloc_bytes",
        "peak_decode_alloc_bytes",
        "tiles_applicable",
        "tiles_failed",
        "status",
        "source",
    ]
    image_rows = collect_rows(records)
    # The group aggregates are computed from the image rows only, so a summary
    # row never contributes to another summary row.
    category_aggregate = bucket_aggregate(image_rows, "category")
    resolution_aggregate = bucket_aggregate(image_rows, "resolution_class")
    corpus_aggregate = merged_aggregate(image_rows)

    # The matched subset: the images where every codec and every baseline was
    # measured. The aggregate over that subset is the comparison to read.
    subset = matched_subset(records)
    subset_rows = collect_rows(subset)
    matched_aggregate_rows = merged_aggregate(subset_rows)
    matched_category_aggregate = bucket_aggregate(subset_rows, "category")
    matched_resolution_aggregate = bucket_aggregate(subset_rows, "resolution_class")

    rows = [dict(row, scope="image", group=row["id"]) for row in image_rows]

    def summary_row(scope: str, group: str, codec: str, node: dict) -> dict:
        return {
            "scope": scope,
            "group": group,
            "id": "",
            "category": group if scope == "category" else "",
            "width": "",
            "height": "",
            "resolution_class": group if scope == "resolution" else "",
            "pixel_format": "",
            "canonical_bytes": node["canonical_bytes"],
            "codec": codec,
            "codec_id": "",
            "bytes": node["bytes"],
            "payload_bytes": "",
            "ratio": node["ratio"],
            "bpp": node["bpp"],
            "raw_fallback_bytes": node.get("raw_fallback_bytes", 0),
            "container_framing_bytes": node.get("container_framing_bytes", 0),
            "covers_all_tiles": node.get("covers_all_tiles"),
            "encode_ms": "",
            "decode_ms": "",
            "exact": node["exact"],
            "framing_bytes": "",
            "model_state_bytes": "",
            "peak_encode_alloc_bytes": "",
            "peak_decode_alloc_bytes": "",
            "tiles_applicable": node["images"],
            "tiles_failed": "",
            "status": "ok",
            "source": "aggregate",
        }

    for group, bucket in category_aggregate.items():
        for codec, node in bucket.items():
            rows.append(summary_row("category", group, codec, node))

    for group, bucket in resolution_aggregate.items():
        for codec, node in bucket.items():
            rows.append(summary_row("resolution", group, codec, node))

    for group, bucket in matched_category_aggregate.items():
        for codec, node in bucket.items():
            rows.append(summary_row("matched-category", group, codec, node))

    for group, bucket in matched_resolution_aggregate.items():
        for codec, node in bucket.items():
            rows.append(summary_row("matched-resolution", group, codec, node))

    for codec, node in corpus_aggregate.items():
        rows.append(summary_row("aggregate", "all", codec, node))

    for codec, node in matched_aggregate_rows.items():
        rows.append(summary_row("matched-aggregate", "matched", codec, node))

    for codec, node in worst_cases(records).items():
        for kind in ("worst_image", "worst_tile"):
            entry = node.get(kind)
            if entry is None:
                continue
            rows.append(
                {
                    "scope": kind,
                    "group": codec,
                    "id": entry.get("id", ""),
                    "category": entry.get("category", ""),
                    "width": "",
                    "height": "",
                    "resolution_class": entry.get("resolution_class", ""),
                    "pixel_format": "",
                    "canonical_bytes": "",
                    "codec": codec,
                    "codec_id": "",
                    "bytes": entry.get("bytes", ""),
                    "payload_bytes": entry.get("payload_bytes", ""),
                    "ratio": entry["ratio"],
                    "bpp": "",
                    "raw_fallback_bytes": "",
                    "container_framing_bytes": "",
                    "covers_all_tiles": "",
                    "encode_ms": "",
                    "decode_ms": "",
                    "exact": "",
                    "framing_bytes": "",
                    "model_state_bytes": "",
                    "peak_encode_alloc_bytes": "",
                    "peak_decode_alloc_bytes": "",
                    "tiles_applicable": "",
                    "tiles_failed": "",
                    "status": "ok",
                    "source": "aggregate",
                }
            )

    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=columns, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            writer.writerow(row)
    return len(rows)


def matched_subset(records: list[dict]) -> list[dict]:
    """The images on which every candidate *and* every available baseline has data.

    A baseline can be skipped: Pillow needs the whole image in memory, so
    the runner skips the baseline of an image above the pixel cap. A codec
    that skips an image and a baseline that skips the same image are not
    comparable, and an aggregate that mixes the two denominators is worse
    than no aggregate at all. This function returns the images where
    nothing was skipped, so one comparison covers one set of images.

    A baseline tool that is *unavailable on this host* — no JPEG XL encoder
    installed — is absent from every image, so it does not disqualify
    anything; it simply contributes no rows to the comparison.
    """
    available: set[str] = set()
    for record in records:
        for name, node in record["baselines"].items():
            if name == "skipped":
                continue
            if "bytes" in node:
                available.add(name)

    subset = []
    for record in records:
        complete = True
        for name in available:
            node = record["baselines"].get(name, {})
            if "bytes" not in node:
                complete = False
                break
        if complete:
            subset.append(record)
    return subset


def matched_aggregate(records: list[dict]) -> dict:
    """The aggregate over the images where nothing was skipped."""
    rows = collect_rows(records)
    return merged_aggregate(rows)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(REPO / "corpus-data"))
    parser.add_argument("--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json"))
    parser.add_argument("--research", default=str(REPO / "target" / "release" / "vpng-research.exe"))
    parser.add_argument("--results", default=str(REPO / "benchmarks" / "results"))
    parser.add_argument("--json-out", default=str(REPO / "benchmarks" / "p3_5_raw_results.json"))
    parser.add_argument("--csv-out", default=str(REPO / "benchmarks" / "p3_5_comparison.csv"))
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--category", action="append", default=[])
    parser.add_argument("--max-baseline-pixels", type=int, default=32_000_000)
    parser.add_argument(
        "--reuse-raw",
        action="store_true",
        help="reuse the saved per-image measurements instead of re-running the harness",
    )
    args = parser.parse_args()

    binary = Path(args.research)
    if not binary.exists():
        raise SystemExit(
            f"vpng-research binary not found: {binary} (cargo build --release --workspace)"
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

    results_dir = Path(args.results)
    results_dir.mkdir(parents=True, exist_ok=True)
    work_root = results_dir / "research-work"
    raw_out = results_dir / "research-raw"

    records = []
    for entry in entries:
        print(f"== {entry['id']} ({entry['category']})", flush=True)
        record = measure_entry(
            entry,
            root,
            work_root / entry["id"],
            binary,
            args.tile_size,
            args.max_baseline_pixels,
            raw_out,
            args.reuse_raw,
        )
        if record is None:
            continue
        best = min(
            (node for node in record["candidates"] if node.get("tiles_applicable")),
            key=lambda node: node.get("file_ratio", node.get("ratio", 99)),
            default=None,
        )
        print(
            "   canonical {:,}  tiles {}  best {} ({:.4f}x)  research {} ms".format(
                record["canonical_bytes"],
                record["tile_count"],
                best["name"] if best else "-",
                best.get("file_ratio", best.get("ratio", 0)) if best else 0,
                record["research_wall_ms"],
            )
        )
        records.append(record)

    rows = collect_rows(records)
    # The per-image candidate entries carry the corrected whole-file figures:
    # the payload, plus the raw fallback for the tiles the codec cannot
    # represent, plus the container framing.
    for record in records:
        for node in record["candidates"]:
            node.update(file_equivalent(record, node))

    # The subset on which every codec and every baseline was measured. A
    # baseline is skipped for an image above the pixel cap, so the headline
    # comparison uses this subset; the per-codec aggregate is reported too,
    # with the image count that went into it.
    subset = matched_subset(records)
    subset_rows = collect_rows(subset)
    matched = merged_aggregate(subset_rows)
    report = {
        "tool": "run_research.py",
        "schema": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "context": {
            "research_binary": str(binary),
            "tile_size": args.tile_size,
            "manifest": str(args.manifest),
            "root": str(root),
            "python": sys.version.split()[0],
        },
        "definitions": {
            "ratio": "encoded bytes divided by canonical pixel bytes; below 1.0 means smaller",
            "file_bytes": (
                "the size a whole file would occupy: the candidate's payload for every "
                "tile it can represent, the raw codec's bytes for every tile it cannot, "
                "plus the container framing (a 64-byte header, a 40-byte directory entry "
                "plus a 4-byte CRC trailer per tile). Baselines are whole files, so the "
                "comparison uses this column."
            ),
            "raw_fallback_bytes": (
                "the canonical bytes of the tiles the candidate cannot represent. A "
                "candidate with a non-zero fallback cannot produce a file on its own."
            ),
            "covers_all_tiles": (
                "whether the candidate could represent every tile of the image. When it "
                "is false, the candidate's ratio is a partial-file figure and the raw "
                "fallback is part of it."
            ),
            "payload_bytes": "the sum of the tile payloads alone, without container framing",
            "bpp": "encoded bytes times eight, divided by canonical pixel bytes",
            "resolution_class": (
                "the smallest ladder entry (512, 1024, 2048, 4096, 8192, 16384) that is "
                "at least the image's longest edge"
            ),
            "memory_note": (
                "peak_*_alloc_bytes are heap bytes tracked by the harness's own global "
                "allocator; they are not process resident set size and exclude the "
                "operating system's own overhead"
            ),
            "experimental_note": (
                "the experimental codecs are research-only. They are not reachable from "
                "get_codec, are not in IMPLEMENTED_CODEC_IDS, and can never appear in a "
                "decodable file"
            ),
            "per_tile_note": (
                "the per_tile section carries the size and exactness of every tile. The "
                "full per-tile record, with the per-tile timings and heap peaks, is kept "
                "verbatim under benchmarks/results/research-raw/"
            ),
            "aggregate_note": (
                "the aggregate section sums each codec over the images that codec was "
                "measured on. A baseline that could not be measured (an image above the "
                "pixel cap) is left out of that baseline's aggregate, so the two "
                "denominators can differ. The matched_aggregate, matched_per_category "
                "and matched_per_resolution sections cover only the images where every "
                "codec and every baseline was measured, and they are the comparison to "
                "read."
            ),
        },
        "candidates": CANDIDATE_ORDER,
        "per_image": [
            {
                "id": record["id"],
                "category": record["category"],
                "width": record["width"],
                "height": record["height"],
                "resolution_class": record["resolution_class"],
                "pixel_format": record["pixel_format"],
                "canonical_bytes": record["canonical_bytes"],
                "canonical_sha256": record["canonical_sha256"],
                "sha256_matches_manifest": record["sha256_matches_manifest"],
                "tile_width": record["tile_width"],
                "tile_height": record["tile_height"],
                "tile_count": record["tile_count"],
                "research_wall_ms": record["research_wall_ms"],
                "candidates": record["candidates"],
                "selection": record["selection"],
                "baselines": record["baselines"],
                "process": record["process"],
            }
            for record in records
        ],
        "per_tile": [
            {"id": record["id"], "tiles": record["tiles"]} for record in records
        ],
        "per_category": bucket_aggregate(rows, "category"),
        "per_resolution": bucket_aggregate(rows, "resolution_class"),
        "matched_per_category": bucket_aggregate(subset_rows, "category"),
        "matched_per_resolution": bucket_aggregate(subset_rows, "resolution_class"),
        "aggregate": merged_aggregate(rows),
        "matched_aggregate": matched,
        "matched_subset": {
            "images": [record["id"] for record in subset],
            "canonical_bytes": sum(record["canonical_bytes"] for record in subset),
            "note": (
                "the images on which every codec and every baseline was measured; "
                "the matched_aggregate, matched_per_category and matched_per_resolution "
                "sections cover exactly these images"
            ),
        },
        "worst_case": worst_cases(records),
    }

    json_path = Path(args.json_out)
    json_path.parent.mkdir(parents=True, exist_ok=True)
    json_path.write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")

    csv_path = Path(args.csv_out)
    count = write_csv(csv_path, records)

    print()
    aggregate = report["aggregate"]
    for codec in sorted(aggregate, key=lambda name: aggregate[name]["ratio"] or 99):
        node = aggregate[codec]
        print(
            f"{codec:<20} bytes {node['bytes']:>12,}  ratio {node['ratio']:.4f}  "
            f"median {node['median_ratio']:.4f}  worst {node['worst_ratio']:.4f}"
        )
    print(f"json: {json_path}")
    print(f"csv:  {csv_path} ({count} rows)")
    return 0


def merged_aggregate(rows: list[dict]) -> dict:
    """The corpus-wide aggregate, one entry per codec."""
    merged: dict = {}
    for row in rows:
        if row["bytes"] is None:
            continue
        node = merged.setdefault(
            row["codec"],
            {
                "bytes": 0,
                "canonical_bytes": 0,
                "images": 0,
                "ratios": [],
                "exact": True,
                "raw_fallback_bytes": 0,
                "covers_all_tiles": True,
            },
        )
        node["bytes"] += row["bytes"]
        node["canonical_bytes"] += row["canonical_bytes"]
        node["images"] += 1
        node["raw_fallback_bytes"] += row.get("raw_fallback_bytes") or 0
        if row.get("covers_all_tiles") is False:
            node["covers_all_tiles"] = False
        if row["ratio"] is not None:
            node["ratios"].append(row["ratio"])
        if row["exact"] is False:
            node["exact"] = False
    for node in merged.values():
        node["ratio"] = round(node["bytes"] / node["canonical_bytes"], 6) if node["canonical_bytes"] else None
        node["bpp"] = round(node["bytes"] * 8 / node["canonical_bytes"], 6) if node["canonical_bytes"] else None
        node["median_ratio"] = round(statistics.median(node["ratios"]), 6) if node["ratios"] else None
        node["worst_ratio"] = max(node["ratios"]) if node["ratios"] else None
        node["best_ratio"] = min(node["ratios"]) if node["ratios"] else None
        node.pop("ratios", None)
    return merged


if __name__ == "__main__":
    raise SystemExit(main())
