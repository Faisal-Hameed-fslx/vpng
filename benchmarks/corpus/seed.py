#!/usr/bin/env python3
"""Assemble a small, provenance-recorded seed corpus.

Image bytes live under the git-ignored corpus root; the manifest keeps id,
category, source URL, licence, native dimensions and the canonical SHA-256.
Every download is optional: a failed fetch is reported and its entry is skipped.

Run from the repository root:
    python benchmarks/corpus/seed.py
"""

import argparse
import json
import shutil
import sys
import time
import urllib.request
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "benchmarks" / "stress"))
sys.path.insert(0, str(HERE))
from canonicalize import convert_image  # noqa: E402
from sources import migrate_manifest  # noqa: E402

USER_AGENT = "VPNG-corpus-seed/0.1 (local benchmark; provenance recorded in benchmarks/corpus_manifest.json)"

DOWNLOADS = [
    {
        "id": "commons-example-1024",
        "category": "photos",
        "path": "photographs/Example.jpg",
        "url": "https://upload.wikimedia.org/wikipedia/commons/a/a9/Example.jpg",
        "license": "CC BY-SA 3.0",
        "attribution": "Wikimedia Commons, File:Example.jpg",
        "notes": "Classic Commons photograph, downloaded at native size (9022 bytes).",
    },
    {
        "id": "commons-cat-1280",
        "category": "photos",
        "path": "photographs/cat_1280.jpg",
        "url": "https://upload.wikimedia.org/wikipedia/commons/thumb/4/4d/Cat_November_2010-1a.jpg/1280px-Cat_November_2010-1a.jpg",
        "license": "CC BY-SA 3.0",
        "attribution": "Wikimedia Commons, File:Cat_November_2010-1a.jpg",
        "notes": "Wikimedia 1280 px rendition; high-frequency fur texture.",
    },
    {
        "id": "commons-starry-night-1280",
        "category": "scans",
        "path": "digital_paintings/starry_night_1280.jpg",
        "url": "https://upload.wikimedia.org/wikipedia/commons/thumb/e/ea/Van_Gogh_-_Starry_Night_-_Google_Art_Project.jpg/1280px-Van_Gogh_-_Starry_Night_-_Google_Art_Project.jpg",
        "license": "Public domain",
        "attribution": "Vincent van Gogh, The Starry Night (1889); Google Art Project scan",
        "notes": "Downscaled by Wikimedia to 1280 px wide; recorded native size is the downloaded size.",
    },
    {
        "id": "commons-great-wave-1280",
        "category": "scans",
        "path": "digital_paintings/great_wave_1280.jpg",
        "url": "https://upload.wikimedia.org/wikipedia/commons/thumb/0/0a/The_Great_Wave_off_Kanagawa.jpg/1280px-The_Great_Wave_off_Kanagawa.jpg",
        "license": "Public domain",
        "attribution": "Katsushika Hokusai, The Great Wave off Kanagawa (c. 1831)",
        "notes": "Ukiyo-e woodblock print; Wikimedia 1280 px rendition.",
    },
]

SYNTHETIC = [
    {"id": "synthetic-gradient-512-rgb8", "category": "textures", "path": "synthetic/gradient_512x512_rgb8.png"},
    {"id": "synthetic-checker-512-rgba8", "category": "textures", "path": "synthetic/checker_512x512_rgba8.png"},
    {"id": "synthetic-noise-256-rgba8", "category": "textures", "path": "synthetic/noise_256x256_rgba8.png"},
    {"id": "synthetic-ui-1024x768-rgb8", "category": "ui", "path": "synthetic/ui_1024x768_rgb8.png"},
]


def download(entry: dict, root: Path) -> bool:
    target = root / entry["path"]
    if target.exists():
        print(f"  present: {entry['path']}")
        return True
    target.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(entry["url"], headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read()
    except Exception as exc:  # noqa: BLE001 - a failed seed fetch is not fatal
        print(f"  FAILED: {entry['path']}: {exc}")
        return False
    if not data:
        print(f"  FAILED: {entry['path']}: empty response")
        return False
    target.write_bytes(data)
    print(f"  downloaded: {entry['path']} ({len(data)} bytes)")
    return True


def generate_synthetic(root: Path) -> None:
    root.joinpath("synthetic").mkdir(parents=True, exist_ok=True)

    width = height = 512
    x = np.arange(width, dtype=np.uint32)[None, :]
    y = np.arange(height, dtype=np.uint32)[:, None]
    gradient = np.stack(
        [
            np.broadcast_to(x * 255 // (width - 1), (height, width)),
            np.broadcast_to(y * 255 // (height - 1), (height, width)),
            np.broadcast_to(((x + y) * 255 // (width + height - 2)), (height, width)),
        ],
        axis=-1,
    ).astype(np.uint8)
    Image.fromarray(gradient).save(root / "synthetic/gradient_512x512_rgb8.png")

    square = 16
    checker = np.zeros((height, width, 4), dtype=np.uint8)
    for row in range(height):
        for col in range(width):
            on = ((row // square) + (col // square)) % 2 == 0
            checker[row, col] = (0x10, 0x20, 0x30, 0xFF) if on else (0xE0, 0xC0, 0xA0, 0xFF)
    Image.fromarray(checker).save(root / "synthetic/checker_512x512_rgba8.png")

    rng = np.random.default_rng(0xC0FFEE)
    noise = rng.integers(0, 256, size=(256, 256, 4), dtype=np.uint8)
    Image.fromarray(noise).save(root / "synthetic/noise_256x256_rgba8.png")

    ui = Image.new("RGB", (1024, 768), (0xF4, 0xF5, 0xF7))
    draw = ImageDraw.Draw(ui)
    draw.rectangle([0, 0, 1024, 36], fill=(0x33, 0x37, 0x3D))
    draw.rectangle([0, 36, 180, 768], fill=(0xE8, 0xEA, 0xED))
    draw.rectangle([200, 60, 1000, 108], fill=(0xFF, 0xFF, 0xFF), outline=(0xC8, 0xCC, 0xD2))
    for line in range(24):
        top = 140 + line * 24
        draw.rectangle([216, top, 216 + 120 + (line * 37) % 600, top + 6], fill=(0x55, 0x5B, 0x66))
        draw.rectangle([216, top + 10, 216 + 60 + (line * 53) % 500, top + 13], fill=(0x9A, 0xA0, 0xAA))
    for item in range(9):
        draw.rectangle([16, 56 + item * 30, 164, 76 + item * 30], fill=(0xF8, 0xF9, 0xFB))
    draw.rectangle([0, 36, 1024, 38], fill=(0xC8, 0xCC, 0xD2))
    ui.save(root / "synthetic/ui_1024x768_rgb8.png")
    print("  generated: 4 synthetic images")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(REPO / "corpus-data"))
    parser.add_argument("--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json"))
    args = parser.parse_args()

    root = Path(args.root)
    root.mkdir(parents=True, exist_ok=True)
    manifest_path = Path(args.manifest)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    existing = {entry["id"]: entry for entry in manifest.get("images", [])}

    print(f"corpus root: {root}")
    print("downloads:")
    downloaded = [entry for entry in DOWNLOADS if download(entry, root)]

    print("synthetic:")
    generate_synthetic(root)

    user_photo = REPO / "1006677.jpg"
    if user_photo.exists():
        target = root / "photographs" / "1006677.jpg"
        target.parent.mkdir(parents=True, exist_ok=True)
        if not target.exists():
            shutil.copy2(user_photo, target)
            print(f"  copied user photo: {target.name} ({target.stat().st_size} bytes)")
    all_entries = list(downloaded)
    all_entries.append(
        {
            "id": "user-photo-15360x8640",
            "category": "photos",
            "path": "photographs/1006677.jpg",
            "source": "user-provided",
            "license": "User-owned; testing use only",
            "notes": "Native 15360x8640 RGB JPEG supplied by the repository owner.",
        }
    )
    for entry in SYNTHETIC:
        all_entries.append(
            {
                **entry,
                "source": "generated by benchmarks/corpus/seed.py",
                "license": "CC0-1.0",
                "notes": "Deterministic synthetic image; not a real photograph or screenshot."
                if entry["category"] in ("textures", "ui")
                else "",
            }
        )

    updated = []
    for entry in all_entries:
        source_path = root / entry["path"]
        if not source_path.exists():
            print(f"  skip (missing): {entry['path']}")
            continue
        tmp = root / ".canonical.tmp"
        meta = convert_image(str(source_path), str(tmp))
        tmp.unlink()
        record = {
            "id": entry["id"],
            "category": entry["category"],
            "path": entry["path"],
            "source": entry.get("source") or entry.get("url"),
            "license": entry["license"],
            "attribution": entry.get("attribution", ""),
            "width": meta["width"],
            "height": meta["height"],
            "pixel_format": meta["pixel_format"],
            "canonical_bytes": meta["bytes"],
            "sha256": meta["sha256"],
            "notes": entry.get("notes", ""),
        }
        if entry.get("url"):
            record["url"] = entry["url"]
        previous = existing.get(entry["id"])
        if previous:
            record = {**previous, **record}
        updated.append(record)
        print(
            f"  recorded: {entry['id']} {meta['width']}x{meta['height']} "
            f"{meta['pixel_format']} {meta['bytes']} bytes sha {meta['sha256'][:12]}"
        )

    # The seed images are merged into the manifest, not written over it: the
    # corpus manifest holds the whole corpus, and re-seeding must not drop
    # the other images.
    merged = {entry["id"]: entry for entry in manifest.get("images", [])}
    for record in updated:
        merged[record["id"]] = record
    images = sorted(merged.values(), key=lambda item: (item["category"], item["id"]))
    manifest["images"] = images
    manifest["total_images"] = len(images)
    manifest["updated"] = time.strftime("%Y-%m-%d")
    manifest = migrate_manifest(manifest)
    # The seed notes are rebuilt each run, so re-seeding is idempotent. The
    # corpus builder's notes are left alone.
    generated_notes = [
        note
        for note in manifest.get("notes", [])
        if "scaffold" not in note and "Seed subset assembled" not in note
        and "Image bytes live under corpus-data" not in note
        and "Built by benchmarks/corpus/build_corpus.py" not in note
    ]
    manifest["notes"] = generated_notes + [
        f"Seed subset recorded on {manifest['updated']}: "
        f"{len(updated)} entries, merged into the corpus manifest.",
        "Image bytes live under corpus-data/ (git-ignored); this manifest is the provenance record.",
    ]
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"manifest updated: {manifest_path} ({len(images)} entries)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())