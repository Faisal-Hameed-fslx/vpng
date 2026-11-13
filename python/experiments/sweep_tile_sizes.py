"""Compression experiment: encode every image in the corpus at several tile sizes.

Writes a JSON report next to the corpus. Requires a built `vpng` binary.

Usage:
    python -m experiments.sweep_tile_sizes --corpus corpus/images --out reports/
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from tools.vpng_cli import encode, run  # noqa: E402

TILE_SIZES = (32, 64, 128, 256, 512, 1024)
IMAGE_SUFFIXES = {".png"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--tile-sizes", type=int, nargs="*", default=list(TILE_SIZES))
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    images = sorted(
        path for path in args.corpus.rglob("*") if path.suffix.lower() in IMAGE_SUFFIXES
    )
    if not images:
        print(f"no images under {args.corpus}", file=sys.stderr)
        return 1

    records = []
    for image in images:
        for tile in args.tile_sizes:
            vpng = args.out / f"{image.stem}.t{tile}.vpng"
            result = encode(str(image), str(vpng), tile_size=tile, json=True)
            record = {
                "image": str(image),
                "tile_size": tile,
                "status": result.status,
                "returncode": result.returncode,
            }
            if result.json:
                record.update(result.json)
            records.append(record)
            print(f"{image.name} tile={tile}: {result.status}")

    report = args.out / "sweep_tile_sizes.json"
    report.write_text(json.dumps(records, indent=2), encoding="utf-8")
    print(f"wrote {report}")

    # Sanity: the CLI must be present and answer --version.
    print("cli:", run("version").stdout.strip())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
