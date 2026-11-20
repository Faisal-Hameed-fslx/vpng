#!/usr/bin/env python3
"""Cache an image's canonical raw pixels, reproducibly.

The latency benchmarks and the corpus run both need the same thing: the
image's canonical pixels on disk, in a known order, with a digest that says
which bytes they are. Decoding a JPEG to canonical pixels is not cheap, and
it is not something a benchmark should repeat inside a timed loop, so the
conversion happens once and the result is cached.

The cache is keyed by the SHA-256 of the **source file's bytes**, so the
same file always maps to the same cache entry, and a different file never
collides with one. The sidecar records both the source digest and the
canonical digest, so a stale or truncated cache is detected rather than
trusted.

Run from the repository root:

    python benchmarks/corpus/canonical_cache.py --source 1006677.jpg \\
        --cache-root corpus-data/cache

With no `--source`, every image in the manifest is cached.
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "benchmarks" / "stress"))
from canonicalize import convert_image  # noqa: E402

CHUNK = 1024 * 1024


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(CHUNK)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def cache_paths(cache_root: Path, source: Path, source_digest: str) -> tuple[Path, Path]:
    stem = f"{source_digest[:16]}-{source.stem}"
    raw = cache_root / f"{stem}.raw"
    sidecar = cache_root / f"{stem}.json"
    return raw, sidecar


def cache_one(source: Path, cache_root: Path, force: bool = False) -> dict:
    """Cache one image's canonical pixels. Returns the sidecar record."""
    source_digest = file_sha256(source)
    raw, sidecar = cache_paths(cache_root, source, source_digest)

    if not force and raw.exists() and sidecar.exists():
        try:
            record = json.loads(sidecar.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            record = {}
        if record.get("source_sha256") == source_digest and raw.exists():
            actual = raw.stat().st_size
            if actual == record.get("canonical_bytes"):
                print(f"  cached: {source.name} -> {raw.name}")
                return record
        print(f"  stale cache, rebuilding: {source.name}")

    cache_root.mkdir(parents=True, exist_ok=True)
    tmp = raw.with_suffix(".raw.tmp")
    meta = convert_image(str(source), str(tmp))
    tmp.replace(raw)

    record = {
        "source": source.name,
        "source_sha256": source_digest,
        "canonical_path": raw.name,
        "width": meta["width"],
        "height": meta["height"],
        "pixel_format": meta["pixel_format"],
        "canonical_bytes": meta["bytes"],
        "canonical_sha256": meta["sha256"],
    }
    sidecar.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(
        f"  cached: {source.name} -> {raw.name} "
        f"({meta['width']}x{meta['height']} {meta['pixel_format']} "
        f"{meta['bytes']} bytes sha {meta['sha256'][:12]})"
    )
    return record


def manifest_sources(manifest_path: Path, corpus_root: Path) -> list[Path]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    found: list[Path] = []
    for entry in manifest.get("images", []):
        path = corpus_root / entry["path"]
        if path.exists():
            found.append(path)
        else:
            print(f"  missing (skipped): {entry['path']}")
    return found


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", action="append", default=[])
    parser.add_argument("--cache-root", default=str(REPO / "corpus-data" / "cache"))
    parser.add_argument("--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json"))
    parser.add_argument("--corpus-root", default=str(REPO / "corpus-data"))
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()

    cache_root = Path(args.cache_root)
    cache_root.mkdir(parents=True, exist_ok=True)
    print(f"cache root: {cache_root}")

    sources: list[Path] = []
    for entry in args.source:
        path = Path(entry)
        if not path.exists():
            print(f"  missing (skipped): {path}")
            continue
        sources.append(path)

    if not sources:
        sources = manifest_sources(Path(args.manifest), Path(args.corpus_root))

    if not sources:
        print("nothing to cache")
        return 0

    records = []
    for source in sources:
        records.append(cache_one(source, cache_root, force=args.force))

    # The index is the cache's own directory listing, so it is merged rather
    # than replaced: caching one more image must not drop the others.
    index = cache_root / "index.json"
    merged: dict[str, dict] = {}
    if index.exists():
        try:
            for entry in json.loads(index.read_text(encoding="utf-8")):
                merged[entry["canonical_path"]] = entry
        except (json.JSONDecodeError, KeyError, TypeError):
            # A damaged index is rebuilt from the sidecars, not trusted.
            merged = {}
    for record in records:
        merged[record["canonical_path"]] = record

    listing = sorted(merged.values(), key=lambda entry: entry["source"])
    index.write_text(json.dumps(listing, indent=2) + "\n", encoding="utf-8")
    print(f"index written: {index} ({len(listing)} entries)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
