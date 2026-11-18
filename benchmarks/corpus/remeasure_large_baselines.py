"""Measure the PNG, WebP and JPEG XL baselines for the corpus's largest image.

Run from the repository root:

    python benchmarks/corpus/remeasure_large_baselines.py

It uses the same helpers as `baselines.py`, so the settings are identical to
the ones the study used. The point is to run the biggest image on its own,
in a fresh process, so the measurement is not competing with anything else
for the host's memory.
"""

import json
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "benchmarks" / "corpus"))

from baselines import jxl_baseline, png_baseline, webp_baseline  # noqa: E402

sys.path.insert(0, str(REPO / "benchmarks" / "corpus"))
from run_corpus import convert_image  # noqa: E402

IMAGE_ID = "user-photo-15360x8640"
WIDTH = 15360
HEIGHT = 8640
FMT = "rgb8"


def main() -> int:
    work = REPO / "benchmarks" / "results" / "large-baseline-work"
    work.mkdir(parents=True, exist_ok=True)
    raw = work / "canonical.raw"
    source = REPO / "corpus-data" / "photographs" / "1006677.jpg"

    print("converting to canonical pixels ...", flush=True)
    started = time.perf_counter()
    meta = convert_image(str(source), str(raw))
    print(f"   {meta['width']}x{meta['height']} {meta['pixel_format']} "
          f"in {time.perf_counter() - started:.1f}s", flush=True)

    with raw.open("rb") as handle:
        pixels = handle.read()
    assert len(pixels) == meta["bytes"], (len(pixels), meta["bytes"])

    out = {}
    for name, fn in (("png", png_baseline), ("webp", webp_baseline), ("jxl", jxl_baseline)):
        print(f"measuring {name} ...", flush=True)
        started = time.perf_counter()
        try:
            node = fn(pixels, WIDTH, HEIGHT, FMT, work)
        except Exception as exc:  # noqa: BLE001
            node = {"error": f"{type(exc).__name__}: {exc}", "tool": name}
        node["wall_s"] = round(time.perf_counter() - started, 1)
        if "bytes" in node and meta["bytes"]:
            node["ratio"] = round(node["bytes"] / meta["bytes"], 5)
        out[name] = node
        print(f"   {json.dumps(node)}", flush=True)

    raw.unlink(missing_ok=True)
    output = REPO / "benchmarks" / "results" / "large-baselines.json"
    output.write_text(json.dumps(out, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
