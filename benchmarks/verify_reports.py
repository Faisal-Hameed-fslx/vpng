"""Check the figures quoted in the milestone reports against the raw data.

This is a one-off verification script for the P3.5 + P4 milestone. It
re-reads the deliverable JSON files and asserts that every number quoted
in the report tables is the number the data actually holds. Run it from
the repository root:

    python benchmarks/verify_reports.py
"""

import csv
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
problems: list[str] = []


def check(condition: bool, message: str) -> None:
    if not condition:
        problems.append(message)


def close(a, b, tol=5e-5) -> bool:
    if a is None or b is None:
        return a is b
    return abs(a - b) <= tol


def main() -> int:
    raw = json.loads((REPO / "benchmarks/p3_5_raw_results.json").read_text(encoding="utf-8"))
    report = (REPO / "docs/P3_5_COMPRESSION_RESEARCH_REPORT.md").read_text(encoding="utf-8")

    # ---- §3.1 the whole corpus (9 images) ------------------------------------
    matched = raw["matched_aggregate"]
    expected_matched = {
        "jxl": (91_196_450, 0.2189),
        "webp": (107_261_534, 0.2575),
        "carc-c": (127_523_075, 0.3062),
        "carc-d": (130_155_579, 0.3125),
        "predictor": (133_500_355, 0.3205),
        "carc-a": (133_500_355, 0.3205),
        "carc-b": (139_231_075, 0.3343),
        "png": (143_484_520, 0.3445),
        "pred-lz77-entropy": (181_170_505, 0.4350),
        "lz77-entropy": (194_163_860, 0.4662),
        "lz77": (333_574_485, 0.8009),
        "rle": (342_347_307, 0.8219),
        "pred-lz77": (346_839_309, 0.8327),
        "palette": (369_128_419, 0.8862),
        "raw": (416_617_300, 1.0002),
    }
    for codec, (bytes_, ratio) in expected_matched.items():
        node = matched.get(codec)
        check(node is not None, f"matched aggregate has no {codec}")
        if node is None:
            continue
        check(
            node["bytes"] == bytes_,
            f"matched {codec}: bytes {node['bytes']} != quoted {bytes_}",
        )
        check(
            close(node["ratio"], ratio),
            f"matched {codec}: ratio {node['ratio']} != quoted {ratio}",
        )
        check(
            node["images"] == 9,
            f"matched {codec}: {node['images']} images, not 9",
        )
        check(
            f"{bytes_:,}" in report,
            f"matched {codec}: the quoted byte count {bytes_:,} is not in the report",
        )

    # The aggregate and the matched aggregate coincide when every baseline
    # was measured on every image.
    aggregate = raw["aggregate"]
    for codec in expected_matched:
        check(
            aggregate.get(codec, {}).get("bytes") == matched.get(codec, {}).get("bytes"),
            f"aggregate and matched_aggregate differ for {codec}",
        )

    # ---- §3.3 matched per-category ------------------------------------------
    category = raw["matched_per_category"]
    expected_category = {
        "photographs": (3, 0.3017, 0.3407, 0.2554, 0.4320, 0.2165),
        "digital_paintings": (2, 0.6841, 0.7270, 0.4987, 0.8240, 0.4495),
        "procedural": (2, 0.0473, 0.0038, 0.0010, 0.0115, 0.0095),
        "screenshots": (1, 0.0277, 0.0023, 0.0003, 0.0040, 0.0006),
        "noise": (1, 1.0170, 1.0021, 0.9984, 1.0157, 1.0827),
    }
    for name, (images, carc_c, png, webp, hybrid, jxl) in expected_category.items():
        bucket = category.get(name)
        check(bucket is not None, f"matched_per_category has no {name}")
        if bucket is None:
            continue
        check(
            bucket["carc-c"]["images"] == images,
            f"{name}: carc-c images {bucket['carc-c']['images']} != quoted {images}",
        )
        for codec, quoted in (("carc-c", carc_c), ("png", png), ("webp", webp),
                              ("jxl", jxl), ("pred-lz77-entropy", hybrid)):
            check(
                close(bucket[codec]["ratio"], quoted),
                f"{name}/{codec}: ratio {bucket[codec]['ratio']} != quoted {quoted}",
            )
        # The report writes the category with a space; the JSON key uses an
        # underscore.
        display = name.replace("_", " ")
        check(f"| {display} |" in report,
              f"{name}: the category row is not in the report")

    # ---- §3.3b matched per-resolution ---------------------------------------
    resolution = raw["matched_per_resolution"]
    expected_resolution = {
        "512": (4, 0.1720, 0.1343, 0.1281, 0.1446),
        "1024": (1, 0.0277, 0.0023, 0.0003, 0.0006),
        "2048": (3, 0.5677, 0.6104, 0.3986, 0.3537),
        "16384": (1, 0.2995, 0.3384, 0.2548, 0.2160),
    }
    for name, (images, carc_c, png, webp, jxl) in expected_resolution.items():
        bucket = resolution.get(name)
        check(bucket is not None, f"matched_per_resolution has no {name}")
        if bucket is None:
            continue
        check(
            bucket["carc-c"]["images"] == images,
            f"resolution {name}: images {bucket['carc-c']['images']} != quoted {images}",
        )
        for codec, quoted in (("carc-c", carc_c), ("png", png), ("webp", webp),
                              ("jxl", jxl)):
            check(
                close(bucket[codec]["ratio"], quoted),
                f"resolution {name}/{codec}: {bucket[codec]['ratio']} != quoted {quoted}",
            )

    # ---- §3.5 the largest image ---------------------------------------------
    big = {r["id"]: r for r in raw["per_image"]}["user-photo-15360x8640"]
    png_bytes = big["baselines"]["png"]["bytes"]
    webp_bytes = big["baselines"]["webp"]["bytes"]
    jxl_bytes = big["baselines"]["jxl"]["bytes"]
    check(
        png_bytes == 134_734_803,
        f"the largest image's PNG is {png_bytes}, not 134734803",
    )
    check(
        webp_bytes == 101_462_746,
        f"the largest image's WebP is {webp_bytes}, not 101462746",
    )
    check(
        jxl_bytes == 85_982_362,
        f"the largest image's JPEG XL is {jxl_bytes}, not 85982362",
    )
    check(
        "134,734,803" in report,
        "the largest image's PNG byte count is not in the report",
    )
    check(
        "101,462,746" in report,
        "the largest image's WebP byte count is not in the report",
    )
    check(
        "85,982,362" in report,
        "the largest image's JPEG XL byte count is not in the report",
    )

    # ---- §3.4 worst cases ----------------------------------------------------
    worst = raw["worst_case"]
    check(
        close(worst["lz77"]["worst_image"]["ratio"], 1.1732),
        f"lz77 worst image ratio {worst['lz77']['worst_image']['ratio']} != 1.1732",
    )
    check(
        close(worst["pred-lz77"]["worst_image"]["ratio"], 1.2347),
        f"pred-lz77 worst image ratio {worst['pred-lz77']['worst_image']['ratio']} != 1.2347",
    )
    check(
        close(worst["lz77"]["worst_tile"]["ratio"], 1.2333),
        f"lz77 worst tile ratio {worst['lz77']['worst_tile']['ratio']} != 1.2333",
    )
    check(
        close(worst["pred-lz77"]["worst_tile"]["ratio"], 1.2487),
        f"pred-lz77 worst tile ratio {worst['pred-lz77']['worst_tile']['ratio']} != 1.2487",
    )

    # ---- the CSV deliverable -------------------------------------------------
    with (REPO / "benchmarks/p3_5_comparison.csv").open(encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle))
    scopes = {row["scope"] for row in rows}
    for scope in ("image", "category", "resolution", "matched-category",
                  "matched-resolution", "aggregate", "matched-aggregate",
                  "worst_image", "worst_tile"):
        check(scope in scopes, f"the CSV has no {scope} rows")
    csv_count = (REPO / "benchmarks/p3_5_comparison.csv").read_text(
        encoding="utf-8"
    ).count("\n")
    check(
        f"{csv_count - 1} rows" in report,
        f"the report says the CSV has a different number of rows than {csv_count - 1}",
    )

    # ---- the P4 memory deliverable ------------------------------------------
    memory = json.loads((REPO / "benchmarks/p4_memory_results.json").read_text(encoding="utf-8"))
    check(memory["summary"] == {"cases": 12, "pass": 12, "fail": 0, "blocked": 0},
          f"the memory summary is {memory['summary']}")
    memory_report = (REPO / "docs/P4_MEMORY_VALIDATION.md").read_text(encoding="utf-8")
    for case in memory["cases"]:
        edit = case["edit"]
        check(
            edit["cache"]["decoded_tiles"] == 1,
            f"{case['id']}: decoded {edit['cache']['decoded_tiles']} tiles, not 1",
        )
        check(
            edit["dirty"]["tile_count"] == 1,
            f"{case['id']}: dirtied {edit['dirty']['tile_count']} tiles, not 1",
        )
        check(
            edit["memory"]["working_set_delta_bytes"] <= memory["context"]["memory_target_bytes"],
            f"{case['id']}: the resident-set delta is over the target",
        )
        check(
            edit["memory"]["heap_peak_bytes"] <= memory["context"]["memory_target_bytes"],
            f"{case['id']}: the heap peak is over the target",
        )
        check(edit["cache"]["within_budget"], f"{case['id']}: the cache is over its budget")
        check(edit["history"]["within_budget"], f"{case['id']}: the history is over its budget")
    check("PASS 12, FAIL 0, BLOCKED 0" in memory_report,
          "the memory report does not state the 12/0/0 result")
    check("0 FAIL, 0 BLOCKED" in memory_report,
          "the memory report does not state the 0/0 failure and blocked counts")

    # ---- the P4 editing deliverable -----------------------------------------
    editing = json.loads((REPO / "benchmarks/p4_editing_results.json").read_text(encoding="utf-8"))
    scenarios = {s["name"]: s for s in editing["scenarios"]}
    check(scenarios["localized-edit"]["cache"]["decoded_tiles"] == 1,
          "the editing benchmark did not decode exactly 1 tile")
    check(scenarios["payload-reuse"]["payload_identity"]["identical_tiles"] == 63,
          "the editing benchmark did not reuse 63 payloads")
    check(scenarios["payload-reuse"]["payload_identity"]["changed_tiles"] == 1,
          "the editing benchmark changed more than 1 payload")
    check(scenarios["save-reopen-digest"]["digests_match"],
          "the editing benchmark's save/reopen digests do not match")
    check(scenarios["undo-redo"]["undo_restores_pixels"],
          "undo did not restore the pixels")
    check(scenarios["undo-redo"]["redo_restores_pixels"],
          "redo did not restore the pixels")
    check(scenarios["abort-preserves-original"]["file_bytes_unchanged"],
          "the abort changed the file")
    for attempt in scenarios["failure-injection"]["attempts"]:
        check(attempt["original_preserved"], f"{attempt['failure_point']}: the original changed")
        check(attempt["no_temporary_files_left"],
              f"{attempt['failure_point']}: a temporary file was left behind")
    check(scenarios["cache-budget"]["within_budget"], "the cache scenario is over its budget")
    check(scenarios["history-budget"]["within_budget"], "the history scenario is over its budget")

    if problems:
        print(f"{len(problems)} problem(s):")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print("all quoted figures match the raw data")
    return 0


if __name__ == "__main__":
    sys.exit(main())
