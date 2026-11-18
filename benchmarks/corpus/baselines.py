#!/usr/bin/env python3
"""Lossless baseline measurements on canonical VPNG pixels.

Every baseline in this module compresses the *same* canonical pixel buffer
that the VPNG codecs see. The settings are fixed so two runs, and two
different scripts, produce comparable numbers:

* PNG — Pillow, ``compress_level=6``.
* WebP — Pillow, lossless, ``quality=100``, ``method=4``.
* JPEG XL — the ``cjxl`` binary, lossless, when it is installed.

This module is the single source of truth for those settings; both
``run_corpus.py`` and ``run_research.py`` use it, so the P3.5 comparison
table and the ordinary corpus report cannot drift apart.
"""

import shutil
import subprocess
import time
from pathlib import Path

from PIL import Image, features

#: The Pillow mode for every canonical pixel format.
PIL_MODES = {
    "gray8": "L",
    "graya8": "LA",
    "rgb8": "RGB",
    "rgba8": "RGBA",
}

#: The Pillow mode for the 16-bit formats Pillow cannot express directly.
#: 16-bit baselines are therefore not produced by Pillow; the callers report
#: that honestly rather than silently widening the samples.
PIL_MODES_16 = {}


def png_baseline(pixels: bytes, width: int, height: int, fmt: str, work: Path) -> dict:
    """Measure a PNG written by Pillow from the canonical pixels."""
    image = Image.frombytes(PIL_MODES[fmt], (width, height), pixels)
    png = work / "baseline.png"
    started = time.perf_counter()
    image.save(png, format="PNG", compress_level=6)
    size = png.stat().st_size
    png.unlink(missing_ok=True)
    return {
        "bytes": size,
        "ms": round((time.perf_counter() - started) * 1000),
        "settings": "Pillow PNG, compress_level=6",
        "tool": "png",
    }


def webp_baseline(pixels: bytes, width: int, height: int, fmt: str, work: Path) -> dict:
    """Measure a lossless WebP written by Pillow from the canonical pixels."""
    if not features.check("webp"):
        return {"error": "Pillow build has no WebP support", "tool": "webp"}
    image = Image.frombytes(PIL_MODES[fmt], (width, height), pixels)
    webp = work / "baseline.webp"
    started = time.perf_counter()
    try:
        image.save(webp, format="WEBP", lossless=True, quality=100, method=4)
        size = webp.stat().st_size
    except Exception as exc:  # noqa: BLE001
        webp.unlink(missing_ok=True)
        return {"error": str(exc), "tool": "webp"}
    webp.unlink(missing_ok=True)
    return {
        "bytes": size,
        "ms": round((time.perf_counter() - started) * 1000),
        "settings": "Pillow WebP lossless, quality=100, method=4",
        "tool": "webp",
    }


def jxl_baseline(pixels: bytes, width: int, height: int, fmt: str, work: Path) -> dict:
    """Measure a lossless JPEG XL written by ``cjxl``, when it is installed.

    JPEG XL is not in the Pillow build, so the pixels are handed to the
    command-line encoder through a PNG intermediate. The PNG is lossless, so
    the encoder sees exactly the canonical pixels.
    """
    cjxl = shutil.which("cjxl")
    if cjxl is None:
        return {"error": "no JPEG XL encoder installed", "tool": "jxl"}
    image = Image.frombytes(PIL_MODES[fmt], (width, height), pixels)
    png = work / "baseline-for-jxl.png"
    jxl = work / "baseline.jxl"
    image.save(png, format="PNG", compress_level=6)
    started = time.perf_counter()
    try:
        proc = subprocess.run(
            [cjxl, str(png), str(jxl), "-d", "0", "--quiet"],
            capture_output=True,
            text=True,
            timeout=1800,
        )
    except subprocess.TimeoutExpired:
        png.unlink(missing_ok=True)
        jxl.unlink(missing_ok=True)
        return {"error": "cjxl timed out", "tool": "jxl"}
    png.unlink(missing_ok=True)
    if proc.returncode != 0:
        jxl.unlink(missing_ok=True)
        return {"error": (proc.stdout + proc.stderr).strip()[:200], "tool": "jxl"}
    size = jxl.stat().st_size
    jxl.unlink(missing_ok=True)
    return {
        "bytes": size,
        "ms": round((time.perf_counter() - started) * 1000),
        "settings": "cjxl -d 0 (lossless), module=default",
        "tool": "jxl",
    }


def measure_baselines(
    raw: Path,
    width: int,
    height: int,
    fmt: str,
    canonical_bytes: int,
    work: Path,
    max_baseline_pixels: int,
) -> dict:
    """Measure every available lossless baseline on the canonical pixels.

    The baselines are skipped for images above the pixel cap: Pillow needs the
    whole image in memory, and so does the comparison. The caller reports the
    skip rather than hiding it.
    """
    if width * height > max_baseline_pixels:
        return {
            "skipped": (
                f"{width * height} pixels exceeds the "
                f"{max_baseline_pixels}-pixel baseline cap"
            )
        }

    with raw.open("rb") as fh:
        pixels = fh.read()
    if len(pixels) != canonical_bytes:
        raise ValueError(
            f"canonical buffer holds {len(pixels)} bytes, expected {canonical_bytes}"
        )

    work.mkdir(parents=True, exist_ok=True)
    out: dict = {}
    out["png"] = png_baseline(pixels, width, height, fmt, work)
    out["webp"] = webp_baseline(pixels, width, height, fmt, work)
    out["jxl"] = jxl_baseline(pixels, width, height, fmt, work)
    for node in out.values():
        if "bytes" in node and canonical_bytes:
            node["ratio"] = round(node["bytes"] / canonical_bytes, 5)
            node["bpp"] = round(node["bytes"] * 8 / canonical_bytes, 6)
    return out
