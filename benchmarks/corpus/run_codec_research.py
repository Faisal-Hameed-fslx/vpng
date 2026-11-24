#!/usr/bin/env python3
"""Measure every codec — the stable ones and the research ones — over the
750-image corpus, on the same canonical pixels and the same tile layout.

This is the measurement the stable codec ids 4-6 are promoted on. The
question it answers is narrow and answerable:

* what does each research codec cost on the corpus, per category?
* what would the encoder's per-tile `auto` selection write if the three
  promoted codecs were stable and therefore selectable?

The rules are the ones the corpus reports already use:

* the denominator is the **canonical pixel bytes**, the same number the P5
  corpus report divides by, so the two reports can be read side by side;
* the tile layout is the same one the corpus uses: a uniform grid at tile
  size 256;
* every figure is a **whole-file** figure: the payload of every tile, plus
  the container framing (a 64-byte header, a 40-byte directory entry per
  tile, and one 4-byte CRC trailer over the directory);
* the median is the middle sample, or the mean of the two middle samples
  when the count is even; the p95 is the **nearest-rank** value;
* exactness is checked by the research harness itself: every tile is
  decoded and compared with the source bytes, and a mismatch fails the
  image.

The selection reproduces the production selector exactly: the smallest
payload wins, and a tie goes to the **lowest codec id**. The three
candidates being promoted are given the ids the specification reserved for
them, so the projection is what a v0.3 encoder would really write.

Nothing here writes a VPNG file with a research codec id. The research
codecs are measured by the `vpng-research` binary, which only ever produces
payloads in memory; the production encoder cannot reference them.

Run from the repository root:

    python benchmarks/corpus/run_codec_research.py
    python benchmarks/corpus/run_codec_research.py --category photos --jobs 2

The run is **resumable**: `--reuse-raw` reuses every measurement already
saved under `benchmarks/results/research-raw` and only measures the images
that are missing.
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

#: The labels the harness used before the three codecs were promoted.
#:
#: The research harness labels a candidate by its *study* name, and the study
#: ran before the promotion, so a measurement saved then carries the old
#: label. Both spellings map to the same candidate.
CANDIDATE_ALIASES = {
    "predictor+dictionary": "pred-lz77",
    "context-entropy": "carc-c",
}

#: The stable id every candidate would carry in a promoted build.
#:
#: The three promoted codecs take the ids the specification reserved for them
#: (ADR-0004); the rest keep their research ids. The selection below uses
#: these ids for its tie-break, so it matches the production selector exactly:
#: the smallest payload wins, and a tie goes to the lowest id.
PROMOTED_IDS = {
    "raw": 0,
    "palette": 1,
    "rle": 2,
    "predictor": 3,
    "lz77": 4,
    "pred-lz77": 5,
    "carc-c": 6,
    "carc-a": 0x80,
    "carc-b": 0x81,
    "carc-d": 0x83,
    "lz77-entropy": 0x92,
    "pred-lz77-entropy": 0x93,
}

#: The codec ids a **v0.1 or v0.2** file may name.
STABLE_IDS = frozenset({0, 1, 2, 3})

#: The codec ids a **v0.3** file may name: the four original ones plus the
#: three promoted in this milestone.
EXTENDED_IDS = frozenset({0, 1, 2, 3, 4, 5, 6})

#: The research candidates, in the order the promotion considered them.
RESEARCH_CANDIDATES = [
    "lz77",
    "pred-lz77",
    "carc-c",
    "carc-a",
    "carc-b",
    "carc-d",
    "lz77-entropy",
    "pred-lz77-entropy",
]

#: The container bytes a file of `tile_count` tiles carries.
HEADER_SIZE = 64
DIRECTORY_ENTRY_SIZE = 40
DIRECTORY_CRC_SIZE = 4


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


def container_framing(tile_count: int) -> int:
    """The container bytes a file of `tile_count` tiles carries.

    A 64-byte header, a 40-byte directory entry per tile, and **one** 4-byte
    CRC32C trailer over the whole directory.
    """
    return HEADER_SIZE + tile_count * DIRECTORY_ENTRY_SIZE + DIRECTORY_CRC_SIZE


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


def canonical_name(name: str) -> str:
    """The study label of a candidate, whichever spelling it arrived with."""
    return CANDIDATE_ALIASES.get(name, name)


def select_per_tile(tiles: list[dict], allowed_ids: frozenset[int]) -> dict:
    """What the encoder's per-tile `auto` selection would write.

    For every tile the smallest applicable payload among the permitted
    codecs wins. A tie goes to the **lowest codec id**, which is exactly what
    the production selector does: it visits the ids in ascending order and a
    candidate must be strictly smaller to replace the incumbent.

    The file's bytes are the payloads plus the container framing.
    """
    payload = 0
    chosen: dict[str, int] = {}
    for tile in tiles:
        best_name = None
        best_bytes = None
        best_id = None
        for result in tile["results"]:
            name = canonical_name(result["name"])
            if result["status"] != "ok":
                continue
            codec_id = PROMOTED_IDS.get(name)
            if codec_id is None or codec_id not in allowed_ids:
                continue
            size = result["payload_bytes"]
            if (
                best_bytes is None
                or size < best_bytes
                or (size == best_bytes and codec_id < best_id)
            ):
                best_name = name
                best_bytes = size
                best_id = codec_id
        if best_name is None:
            # Every candidate refused this tile. The raw codec is always
            # applicable, so this cannot happen; it is a hard error rather
            # than a silent fallback.
            raise RuntimeError("no candidate could represent a tile")
        payload += best_bytes
        chosen[best_name] = chosen.get(best_name, 0) + 1
    framing = container_framing(len(tiles))
    return {
        "payload_bytes": payload,
        "framing_bytes": framing,
        "file_bytes": payload + framing,
        "tiles_by_codec": chosen,
    }


def measure_entry(
    entry: dict,
    root: Path,
    work: Path,
    binary: Path,
    cache: dict[str, Path],
    tile_size: int,
    raw_cache: Path,
    reuse: bool,
) -> dict | None:
    """Measure one corpus image with every candidate codec."""
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
    research_json = work / "research.json"
    cached_json = raw_cache / f"{entry['id']}.json"

    started = time.perf_counter()
    if reuse and cached_json.exists():
        document = json.loads(cached_json.read_text(encoding="utf-8"))
        print("   reusing the saved measurement", flush=True)
    else:
        # The host is memory-pressured and the harness allocates a large
        # working set per candidate, so an allocation can fail outright. A
        # retry clears the transient pressure; the retries are counted and
        # reported, never hidden.
        attempts = 0
        while True:
            attempts += 1
            proc = subprocess.run(
                [
                    str(binary),
                    "--input", str(raw),
                    "--width", str(width),
                    "--height", str(height),
                    "--format", fmt,
                    "--tile-size", str(tile_size),
                    "--output", str(research_json),
                ],
                capture_output=True,
                text=True,
                timeout=7200,
            )
            if proc.returncode == 0:
                break
            if attempts >= 3:
                raise RuntimeError(
                    f"vpng-research failed for {entry['id']} after {attempts} attempts "
                    f"(exit {proc.returncode}): "
                    f"{(proc.stdout + proc.stderr).strip()[:400]}"
                )
            print(
                f"   attempt {attempts} failed (exit {proc.returncode}); retrying",
                flush=True,
            )
        document = json.loads(research_json.read_text(encoding="utf-8"))
        raw_cache.mkdir(parents=True, exist_ok=True)
        cached_json.write_text(json.dumps(document) + "\n", encoding="utf-8")
    wall_ms = round((time.perf_counter() - started) * 1000)

    tiles = document["tiles"]
    canonical = meta["bytes"]

    record = {
        "id": entry["id"],
        "category": entry["category"],
        "synthetic": bool(entry.get("synthetic", False)),
        "width": width,
        "height": height,
        "pixel_format": fmt,
        "canonical_bytes": canonical,
        "canonical_sha256": meta["sha256"],
        "tile_count": len(tiles),
        "tile_size": tile_size,
        "research_wall_ms": wall_ms,
        "candidates": {},
    }

    # Every candidate on its own: the payload of the tiles it can represent,
    # and the raw codec's bytes for the tiles it cannot.
    for node in document["per_image"]:
        name = canonical_name(node["name"])
        payload = node.get("payload_bytes", 0)
        applicable = node.get("canonical_bytes", 0)
        fallback = max(canonical - applicable, 0)
        framing = container_framing(len(tiles))
        record["candidates"][name] = {
            "payload_bytes": payload,
            "tiles_applicable": node.get("tiles_applicable", 0),
            "tiles_failed": node.get("tiles_failed", 0),
            "exact": bool(node.get("exact", False)),
            "raw_fallback_bytes": fallback,
            "file_bytes": payload + fallback + framing,
            "file_ratio": round((payload + fallback + framing) / canonical, 6),
            "encode_ms": round(node.get("encode_ms", 0.0), 3),
            "decode_ms": round(node.get("decode_ms", 0.0), 3),
        }

    # The two selections: today's stable `auto`, and the `auto` that would
    # see the promoted codecs as well.
    stable = select_per_tile(tiles, STABLE_IDS)
    extended = select_per_tile(tiles, EXTENDED_IDS)
    for name, selection in (("stable_auto", stable), ("extended_auto", extended)):
        record[name] = {
            **selection,
            "file_ratio": round(selection["file_bytes"] / canonical, 6),
        }

    # The single best research candidate, chosen by the file it would make.
    best_research = None
    for name in RESEARCH_CANDIDATES:
        node = record["candidates"].get(name)
        if node is None or not node["exact"]:
            continue
        if best_research is None or node["file_bytes"] < best_research["file_bytes"]:
            best_research = {"name": name, **node}
    record["best_research"] = best_research

    if not cached and not reuse:
        raw.unlink(missing_ok=True)
    research_json.unlink(missing_ok=True)
    try:
        work.rmdir()
    except OSError:
        pass

    delta = extended["file_bytes"] - stable["file_bytes"]
    record["delta_bytes"] = delta
    record["extended_smaller"] = delta < 0
    record["extended_larger"] = delta > 0
    record["extended_equal"] = delta == 0
    return record


def summarise(records: list[dict]) -> dict:
    """The distribution the report quotes."""

    def bucket(items: list[dict]) -> dict:
        row: dict = {"count": len(items)}
        for name in ("stable_auto", "extended_auto"):
            sizes = [item[name]["file_bytes"] for item in items]
            ratios = [item[name]["file_ratio"] for item in items]
            row[name] = {
                "total_bytes": sum(sizes),
                "ratio": distribution(ratios),
            }
        deltas = [item["delta_bytes"] for item in items]
        row["delta_bytes"] = distribution(deltas)
        row["extended_smaller"] = sum(1 for item in items if item["extended_smaller"])
        row["extended_larger"] = sum(1 for item in items if item["extended_larger"])
        row["extended_equal"] = sum(1 for item in items if item["extended_equal"])
        # Every candidate on its own.
        for name in CANDIDATE_ORDER:
            applicable = [
                item
                for item in items
                if name in item["candidates"] and item["candidates"][name]["exact"]
            ]
            if not applicable:
                row[name] = {"count": 0, "exact": 0}
                continue
            ratios = [item["candidates"][name]["file_ratio"] for item in applicable]
            row[name] = {
                "count": len(applicable),
                "exact": len(applicable),
                "total_bytes": sum(
                    item["candidates"][name]["file_bytes"] for item in applicable
                ),
                "ratio": distribution(ratios),
            }
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
        "--research", default=str(REPO / "target" / "release" / "vpng-research.exe")
    )
    parser.add_argument(
        "--cache-root", default=str(REPO / "corpus-data" / "cache")
    )
    parser.add_argument(
        "--raw-cache", default=str(REPO / "benchmarks" / "results" / "research-raw")
    )
    parser.add_argument(
        "--output", default=str(REPO / "benchmarks" / "p8_codec_corpus_results.json")
    )
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--category", action="append", default=[])
    parser.add_argument("--only", action="append", default=[])
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

    cache = cache_index(Path(args.cache_root))
    raw_cache = Path(args.raw_cache)
    work_root = REPO / "benchmarks" / "results" / "research-work"

    started = time.perf_counter()
    records: list[dict] = []

    def run(entry: dict) -> dict | None:
        return measure_entry(
            entry,
            root,
            work_root / entry["id"],
            binary,
            cache,
            args.tile_size,
            raw_cache,
            args.reuse_raw,
        )

    if args.jobs > 1:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            for entry in entries:
                print(f"== {entry['id']} ({entry['category']})", flush=True)
                record = pool.submit(run, entry).result()
                if record is not None:
                    records.append(record)
                    _print_row(record)
    else:
        for entry in entries:
            print(f"== {entry['id']} ({entry['category']})", flush=True)
            record = run(entry)
            if record is not None:
                records.append(record)
                _print_row(record)

    summary = summarise(records)
    document = {
        "tool": "run_codec_research.py",
        "schema": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "context": {
            "research_binary": str(binary),
            "tile_size": args.tile_size,
            "manifest": str(args.manifest),
            "root": str(root),
            "python": sys.version.split()[0],
            "jobs": args.jobs,
            "parameters": {
                "denominator": "canonical pixel bytes",
                "tile_size": args.tile_size,
                "stable_codec_ids": sorted(STABLE_IDS),
                "promoted_codec_ids": sorted(EXTENDED_IDS),
                "promoted_id_by_candidate": PROMOTED_IDS,
                "research_candidates": RESEARCH_CANDIDATES,
                "candidate_order": CANDIDATE_ORDER,
                "tie_break": (
                    "the smallest applicable payload; a tie goes to the "
                    "lowest codec id, exactly as the production selector does"
                ),
            },
        },
        "definitions": {
            "file_bytes": (
                "the size a whole file would occupy: the payload of every tile, "
                "plus the container framing (a 64-byte header, a 40-byte "
                "directory entry per tile, and one 4-byte CRC trailer over the "
                "directory)"
            ),
            "stable_auto": (
                "the per-tile selection over the v0.1 and v0.2 codec ids only "
                "(raw, palette, rle, predictor) — what the encoder writes "
                "today"
            ),
            "extended_auto": (
                "the per-tile selection over the v0.3 codec ids: the four "
                "original codecs plus the three promoted in this milestone "
                "(lz77, predictor+dictionary, context-entropy) — what the "
                "encoder writes once the promotion lands"
            ),
            "ratio": "file bytes divided by canonical pixel bytes; below 1.0 is smaller",
            "exact": "the harness decoded every tile and compared it with the source bytes",
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
        "stable auto {} bytes  extended auto {} bytes  delta {} bytes".format(
            f"{overall['stable_auto']['total_bytes']:,}",
            f"{overall['extended_auto']['total_bytes']:,}",
            f"{overall['extended_auto']['total_bytes'] - overall['stable_auto']['total_bytes']:,}",
        )
    )
    print(
        "smaller {}  larger {}  equal {}".format(
            overall["extended_smaller"],
            overall["extended_larger"],
            overall["extended_equal"],
        )
    )
    print(f"wrote {output}")
    return 0


def _print_row(record: dict) -> None:
    print(
        "   canonical {:>12,}  stable {:>12,}  extended {:>12,}  delta {:>+10,}  {}".format(
            record["canonical_bytes"],
            record["stable_auto"]["file_bytes"],
            record["extended_auto"]["file_bytes"],
            record["delta_bytes"],
            " ".join(
                f"{name}={count}"
                for name, count in sorted(record["extended_auto"]["tiles_by_codec"].items())
            ),
        ),
        flush=True,
    )


if __name__ == "__main__":
    sys.exit(main())
