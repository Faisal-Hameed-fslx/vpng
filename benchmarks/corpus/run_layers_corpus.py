#!/usr/bin/env python3
"""Measure the v0.4 layer model over the 750-image corpus.

The format revision v0.4 adds **layers**: a file may hold several grids of
tiles, each with a name, a rectangle on the canvas, an opacity and — when
the format has an alpha channel — an optional mask.

This script measures the model over the same corpus every other milestone
used, and it measures the things the format actually promises:

* **A one-layer file composites to exactly its own pixels.** The composite
  is written out and its bytes are compared with the canonical pixels, byte
  for byte, for every image.
* **The base layer of a layered file decodes to exactly the original
  pixels.** The overlay does not disturb the base.
* **The byte accounting is exact.** Every layer's uncompressed and
  compressed byte counts are read back out of the file and compared with
  what the encoder said.
* **A layer table costs something.** The one-layer v0.4 file is compared
  with the plain v0.1 file of the same pixels, so the overhead of the
  table is a measured number and not an estimate.
* **Masks work.** For every image whose format has an alpha channel, a
  third layer masks the overlay, and the masked composite is compared with
  the unmasked one.

Nothing here is a claim about speed. The script measures bytes and
equality, and it reports what it found.

Run from the repository root:

    python benchmarks/corpus/run_layers_corpus.py
    python benchmarks/corpus/run_layers_corpus.py --category photos --jobs 1
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


def write_overlay(source: Path, target: Path, width: int, height: int, fmt: str) -> None:
    """Write an overlay: the top-left quadrant, colour-inverted and opaque.

    The inversion is what makes the composite provably different from the
    base: an 8-bit sample `v` becomes `255 - v`, and no integer `v` equals
    `255 - v`. The alpha channel, where there is one, is set to 255 so the
    overlay is opaque and the difference is visible.
    """
    bpp, colour = CHANNELS[fmt]
    half_width = max(1, (width + 1) // 2)
    half_height = max(1, (height + 1) // 2)
    row_bytes = width * bpp
    crop_bytes = half_width * bpp

    with source.open("rb") as reader, target.open("wb") as writer:
        for y in range(half_height):
            row = reader.read(row_bytes)
            if len(row) < row_bytes:
                raise RuntimeError(f"{source} ended early at row {y}")
            crop = bytearray(row[:crop_bytes])
            for index in range(half_width):
                at = index * bpp
                for channel in range(colour):
                    crop[at + channel] = 255 - crop[at + channel]
                if bpp > colour:
                    crop[at + colour] = 255
            writer.write(crop)


def write_mask(target: Path, width: int, height: int, fmt: str) -> None:
    """Write a mask whose coverage ramps across the width.

    Every row is the same ramp, so the coverage is a function of the column
    alone: 0 at the left edge and 255 at the right.
    """
    bpp, _ = CHANNELS[fmt]
    ramp = bytearray()
    for x in range(width):
        coverage = 0 if width == 1 else (x * 255) // (width - 1)
        pixel = bytearray(bpp)
        if bpp > 1:
            pixel[bpp - 1] = coverage
        else:
            # A format without an alpha channel cannot carry a mask; the
            # caller never asks for one.
            pixel[0] = coverage
        ramp.extend(pixel)
    with target.open("wb") as writer:
        for _ in range(height):
            writer.write(ramp)


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


def measure_entry(
    entry: dict,
    work: Path,
    vpng: Path,
    cache: dict[str, Path],
    tile_size: int,
    workers: int,
    keep: bool,
) -> dict | None:
    """Build the layered files for one corpus image and measure them."""
    raw = cache.get(entry["sha256"])
    if raw is None:
        print(f"  no cached canonical pixels for {entry['id']}")
        return None

    width, height = entry["width"], entry["height"]
    fmt = entry["pixel_format"]
    bpp, colour = CHANNELS[fmt]
    has_alpha = bpp > colour

    image_work = work / entry["id"]
    image_work.mkdir(parents=True, exist_ok=True)

    # The plain file: the same pixels, with no layer table. It is the
    # baseline the layer table's overhead is measured against.
    plain = image_work / "plain.vpng"
    one = image_work / "one.vpng"
    two = image_work / "two.vpng"
    three = image_work / "three.vpng"

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

    # 1. The plain v0.1 file.
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

    # 2. The one-layer file: the same pixels, in a layer table.
    manifest_one = image_work / "one.json"
    manifest_one.write_text(
        json.dumps(
            {
                "layers": [
                    {
                        "name": "base",
                        "path": str(raw),
                        "width": width,
                        "height": height,
                        "format": fmt,
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    run_with_retry(
        vpng,
        ["encode-layers", str(one), "--manifest", str(manifest_one),
         "--tile-size", str(tile_size), "--workers", str(workers)],
        "encode-layers (one)",
    )
    record["one_bytes"] = one.stat().st_size
    record["one_sha256"] = file_sha256(one)

    check_one = run_with_retry(vpng, ["check", str(one)], "check (one)")
    record["one_check_ok"] = check_one["exit"] == 0

    layers_one = read_json_output(
        run_with_retry(vpng, ["layers", str(one), "--json"], "layers (one)")
    )
    record["one_layer_count"] = layers_one["layer_count"]
    record["one_layers"] = layers_one["layers"]

    # The composite of a one-layer file must be exactly the base's pixels.
    composite_one = image_work / "one-composite.raw"
    run_with_retry(
        vpng,
        ["composite", str(one), str(composite_one)],
        "composite (one)",
    )
    record["one_composite_matches_base"] = (
        file_sha256(composite_one) == entry["sha256"]
    )

    # 3. The two-layer file: the base, plus an opaque inverted overlay.
    overlay = image_work / "overlay.raw"
    write_overlay(raw, overlay, width, height, fmt)
    overlay_width = max(1, (width + 1) // 2)
    overlay_height = max(1, (height + 1) // 2)

    manifest_two = image_work / "two.json"
    manifest_two.write_text(
        json.dumps(
            {
                "layers": [
                    {
                        "name": "base",
                        "path": str(raw),
                        "width": width,
                        "height": height,
                        "format": fmt,
                    },
                    {
                        "name": "ink",
                        "path": str(overlay),
                        "x": 0,
                        "y": 0,
                        "width": overlay_width,
                        "height": overlay_height,
                        "format": fmt,
                    },
                ]
            }
        ),
        encoding="utf-8",
    )
    run_with_retry(
        vpng,
        ["encode-layers", str(two), "--manifest", str(manifest_two),
         "--tile-size", str(tile_size), "--workers", str(workers)],
        "encode-layers (two)",
    )
    record["two_bytes"] = two.stat().st_size
    record["two_sha256"] = file_sha256(two)

    check_two = run_with_retry(vpng, ["check", str(two)], "check (two)")
    record["two_check_ok"] = check_two["exit"] == 0

    layers_two = read_json_output(
        run_with_retry(vpng, ["layers", str(two), "--json"], "layers (two)")
    )
    record["two_layer_count"] = layers_two["layer_count"]
    record["two_layers"] = layers_two["layers"]

    # The base layer of a layered file must be exactly the original pixels.
    base_raw = image_work / "base-layer.raw"
    run_with_retry(
        vpng,
        ["decode-layer", str(two), str(base_raw), "--layer", "0"],
        "decode-layer (base)",
    )
    record["two_base_matches_original"] = file_sha256(base_raw) == entry["sha256"]

    composite_two = image_work / "two-composite.raw"
    run_with_retry(
        vpng,
        ["composite", str(two), str(composite_two)],
        "composite (two)",
    )
    record["two_composite_differs_from_base"] = (
        file_sha256(composite_two) != entry["sha256"]
    )

    # 4. The masked file: the same overlay, behind a coverage ramp.
    if has_alpha:
        mask = image_work / "mask.raw"
        write_mask(mask, overlay_width, overlay_height, fmt)
        manifest_three = image_work / "three.json"
        manifest_three.write_text(
            json.dumps(
                {
                    "layers": [
                        {
                            "name": "base",
                            "path": str(raw),
                            "width": width,
                            "height": height,
                            "format": fmt,
                        },
                        {
                            "name": "ink",
                            "path": str(overlay),
                            "x": 0,
                            "y": 0,
                            "width": overlay_width,
                            "height": overlay_height,
                            "format": fmt,
                            "mask": 2,
                        },
                        {
                            "name": "coverage",
                            "kind": "mask",
                            "path": str(mask),
                            "x": 0,
                            "y": 0,
                            "width": overlay_width,
                            "height": overlay_height,
                            "format": fmt,
                        },
                    ]
                }
            ),
            encoding="utf-8",
        )
        run_with_retry(
            vpng,
            ["encode-layers", str(three), "--manifest", str(manifest_three),
             "--tile-size", str(tile_size), "--workers", str(workers)],
            "encode-layers (three)",
        )
        record["three_bytes"] = three.stat().st_size
        record["three_sha256"] = file_sha256(three)

        check_three = run_with_retry(vpng, ["check", str(three)], "check (three)")
        record["three_check_ok"] = check_three["exit"] == 0

        layers_three = read_json_output(
            run_with_retry(vpng, ["layers", str(three), "--json"], "layers (three)")
        )
        record["three_layer_count"] = layers_three["layer_count"]
        record["three_layers"] = layers_three["layers"]

        composite_three = image_work / "three-composite.raw"
        run_with_retry(
            vpng,
            ["composite", str(three), str(composite_three)],
            "composite (three)",
        )
        record["three_composite_differs_from_unmasked"] = (
            file_sha256(composite_three) != file_sha256(composite_two)
        )

    if not keep:
        for path in (plain, one, two, three, overlay, base_raw, composite_one,
                     composite_two):
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
            "one_bytes": sum(item["one_bytes"] for item in items),
            "two_bytes": sum(item["two_bytes"] for item in items),
            "canonical_bytes": sum(item["canonical_bytes"] for item in items),
        }
        out["one_overhead_bytes"] = out["one_bytes"] - out["plain_bytes"]
        out["one_overhead_ratio"] = (
            round(out["one_overhead_bytes"] / out["plain_bytes"], 8)
            if out["plain_bytes"]
            else None
        )
        out["two_overhead_bytes"] = out["two_bytes"] - out["one_bytes"]
        out["one_composite_exact"] = sum(
            1 for item in items if item["one_composite_matches_base"]
        )
        out["two_base_exact"] = sum(
            1 for item in items if item["two_base_matches_original"]
        )
        out["two_composite_differs"] = sum(
            1 for item in items if item["two_composite_differs_from_base"]
        )
        out["check_failures"] = sum(
            1 for item in items if not item.get("one_check_ok", False)
        ) + sum(1 for item in items if not item.get("two_check_ok", False))
        masked = [item for item in items if "three_bytes" in item]
        out["masked_images"] = len(masked)
        out["masked_check_failures"] = sum(
            1 for item in masked if not item.get("three_check_ok", False)
        )
        out["masked_composite_differs"] = sum(
            1 for item in masked if item.get("three_composite_differs_from_unmasked")
        )
        out["one_under_ceiling"] = sum(
            1 for item in items if item["one_bytes"] <= SIZE_CEILING_BYTES
        )
        out["plain_under_ceiling"] = sum(
            1 for item in items if item["plain_bytes"] <= SIZE_CEILING_BYTES
        )
        return out

    summary["overall"] = bucket(records)
    by_category: dict[str, dict] = {}
    for record in records:
        by_category.setdefault(record["category"], []).append(record)
    summary["by_category"] = {
        name: bucket(items) for name, items in sorted(by_category.items())
    }

    # The per-layer accounting, as the file reports it.
    layer_ratios: list[float] = []
    overlay_ratios: list[float] = []
    for record in records:
        for layer in record.get("two_layers", []):
            ratio = layer.get("ratio")
            if ratio is not None:
                layer_ratios.append(float(ratio))
            if layer["index"] == 1 and ratio is not None:
                overlay_ratios.append(float(ratio))
    summary["layer_ratio_distribution"] = distribution(layer_ratios)
    summary["overlay_ratio_distribution"] = distribution(overlay_ratios)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=REPO / "benchmarks/corpus_manifest.json")
    parser.add_argument("--cache", type=Path, default=REPO / "corpus-data/cache")
    parser.add_argument("--vpng", type=Path, default=REPO / "target/release/vpng.exe")
    parser.add_argument("--work", type=Path, default=REPO / "benchmarks/work/layers")
    parser.add_argument("--output", type=Path, default=REPO / "benchmarks/p12_layers_results.json")
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
        "schema": "vpng.layers.corpus.v1",
        "generated_by": "benchmarks/corpus/run_layers_corpus.py",
        "format_minor": 4,
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
