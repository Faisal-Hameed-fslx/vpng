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


def check_p5() -> None:
    """Check the P5 report against the two viewing result files."""
    report = (REPO / "docs/P5_REPORT.md").read_text(encoding="utf-8")
    big = json.loads((REPO / "benchmarks/p5_view_results.json").read_text(encoding="utf-8"))
    small = json.loads((REPO / "benchmarks/p5_view_results_small.json").read_text(encoding="utf-8"))

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the report does not contain the figure {text!r}")

    for data, label in ((big, "16K"), (small, "2K")):
        r = data["report"]
        s = r["scenarios"]
        fv = s["first_viewport"]

        # The file is the size the report says it is.
        quoted(r["file"]["file_size_bytes"], ",")
        quoted(r["file"]["tile_count"], ",")
        quoted(fv["bytes_read"], ",")
        quoted(s["sequential_baseline"]["bytes_read_from_file"], ",")
        # The header and the directory are the difference between what the
        # baseline read and what the pool's workers read.
        header = s["sequential_baseline"]["bytes_read_from_file"] - fv["bytes_read"]
        quoted(header, ",")

        # The first viewport, and the baseline it is measured against.
        quoted(fv["first_viewport_ms"], ".2f")
        quoted(fv["open"]["elapsed_ms"], ".2f")
        quoted(s["sequential_baseline"]["elapsed_ms"], ".2f")
        ratio = s["sequential_baseline"]["elapsed_ms"] / fv["first_viewport_ms"]
        quoted(round(ratio, 2), ".2f")

        # The acceptance properties, not just the figures.
        check(fv["only_viewport_tiles_decoded"], f"{label}: more than the viewport's tiles were decoded")
        check(
            fv["tiles_decoded"] == fv["viewport"]["visible_tiles"],
            f"{label}: the decoded tile count is not the viewport's tile count",
        )
        check(
            fv["pixel_identity"]["different_from_sequential"] == 0,
            f"{label}: the parallel pixels differ from the sequential ones",
        )
        check(
            fv["pixel_identity"]["identical_to_sequential"] == fv["viewport"]["visible_tiles"],
            f"{label}: not every viewport tile was compared",
        )
        check(
            s["edit_and_save"]["payload_identity"]["untouched_payloads_byte_identical"],
            f"{label}: the untouched payloads changed",
        )
        check(s["edit_and_save"]["pixels_read_back_exact"], f"{label}: the pixels are not exact")
        quoted(fv["memory"]["heap_peak_bytes"], ",")
        quoted(fv["memory"]["working_set_peak_bytes"], ",")
        quoted(fv["cache"]["peak_bytes"], ",")

        # The prefetch sweep: every radius, including the losing ones.
        comparison = s["prefetch_comparison"]
        for radius in comparison["radii"]:
            run = comparison["runs"][str(radius)]
            quoted(run["total_ms"], ".2f")
            quoted(run["visible_hit_rate"], ".4f")
            quoted(run["tiles_decoded"], "d")
            quoted(run["cancelled"], "d")
        check("best_radius" in comparison, f"{label}: the sweep has no best radius")

        # The two pan scenarios.
        for key in ("pan_without_prefetch", "pan_with_prefetch"):
            pan = s[key]
            quoted(pan["total_ms"], ".2f")
            quoted(pan["mean_ms"], ".2f")
            quoted(pan["max_ms"], ".2f")
            quoted(pan["tiles_decoded"], "d")
            quoted(pan["scheduler"]["cancellation_rate"], ".4f")
            quoted(pan["recovered_results"], "d")
            quoted(pan["memory"]["working_set_peak_bytes"], ",")

        # The memory figures for the whole run.
        quoted(data["process"]["peak_working_set_bytes"], ",")
        quoted(data["process"]["peak_heap_bytes"], ",")
        check(
            data["process"]["peak_working_set_bytes"] <= 64 * 1024 * 1024,
            f"{label}: the resident set exceeds the 64 MB target",
        )
        check(
            data["process"]["peak_heap_bytes"] <= 64 * 1024 * 1024,
            f"{label}: the heap exceeds the 64 MB target",
        )
        check(fv["cache"]["within_budget"], f"{label}: the viewing cache is over its budget")

    # The two workloads disagree about prefetch, and the report says so.
    check(big["report"]["scenarios"]["prefetch_comparison"]["prefetch_reduced_total_latency"],
          "the 16K sweep no longer shows prefetch helping")
    check(not small["report"]["scenarios"]["prefetch_comparison"]["prefetch_reduced_total_latency"],
          "the 2K sweep no longer shows prefetch not helping")
    check(big["report"]["scenarios"]["prefetch_comparison"]["best_radius"] == 2,
          "the best radius at 16K is not 2")
    check(small["report"]["scenarios"]["prefetch_comparison"]["best_radius"] == 0,
          "the best radius at 2K is not 0")
    check("does not help" in report or "does not always" in report,
          "the report does not record that prefetch can fail to help")


def check_encode() -> None:
    """Check the parallel-encoder report against the raw data."""
    report = (REPO / "docs/P5_ENCODER_REPORT.md").read_text(encoding="utf-8")
    raw = json.loads((REPO / "benchmarks/p5_encode_results.json").read_text(encoding="utf-8"))
    r = raw["report"]
    streaming = r["scenarios"]["streaming"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the encoder report does not contain the figure {text!r}")

    # The central claim: every worker count wrote the same file.
    check(
        streaming["all_worker_counts_identical"],
        "the worker counts did not all produce the same file",
    )
    digests = {run["sha256"] for run in streaming["runs"]}
    check(len(digests) == 1, "the streamed files have different digests")
    check(streaming["sha256"] in digests, "the summary digest does not match a run")

    # Every run's figures must be quoted in the report.
    for run in streaming["runs"]:
        quoted(run["encode_ms"], ".2f")
        quoted(run["speedup_vs_one_worker"], ".2f")
        quoted(run["file_size_bytes"], ",")
        quoted(run["sha256"], "")

    # The batch encoder and the streaming writer must agree.
    agreement = r["scenarios"]["batch_agreement"]
    check(agreement["identical"], "the batch encoder and the streaming writer disagree")
    check(
        agreement["batch_sha256"] == agreement["streaming_sha256"],
        "the batch and streaming digests differ",
    )
    check(
        agreement["batch_file_size_bytes"] == agreement["streaming_file_size_bytes"],
        "the batch and streaming file sizes differ",
    )
    quoted(agreement["batch_ms"], ".2f")
    quoted(agreement["streaming_ms"], ".2f")
    quoted(agreement["batch_file_size_bytes"], ",")
    quoted(agreement["batch_sha256"], "")

    # The memory the streaming encoder used must stay inside the target. The
    # process-wide peak is NOT checked: it includes the agreement scenario,
    # whose batch encoder deliberately holds a whole image in memory.
    streaming_memory = streaming["memory"]
    check(
        streaming_memory["peak_working_set_bytes"] <= 64 * 1024 * 1024,
        "the streaming encoder's resident set exceeds the 64 MB target",
    )
    check(
        streaming_memory["peak_heap_bytes"] <= 64 * 1024 * 1024,
        "the streaming encoder's heap exceeds the 64 MB target",
    )
    quoted(streaming_memory["peak_working_set_bytes"], ",")
    quoted(streaming_memory["peak_heap_bytes"], ",")

    # The report must say that the digests are the proof.
    check(
        "sha256" in report.lower() or "digest" in report.lower(),
        "the encoder report does not mention the digest",
    )


def check_bench() -> None:
    """Check the multi-run photographic baseline report against the raw data."""
    report = (REPO / "docs/P5_BENCH_REPORT.md").read_text(encoding="utf-8")
    raw = json.loads((REPO / "benchmarks/p5_bench_results.json").read_text(encoding="utf-8"))
    r = raw["report"]
    s = r["scenarios"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the benchmark report does not contain the figure {text!r}")

    # The source is the photograph, and the report says how big it is.
    quoted(r["source"]["canonical_bytes"], ",")
    quoted(r["source"]["width"], ",")
    quoted(r["source"]["height"], ",")
    check(
        r["source"]["canonical_bytes"] == r["source"]["file_bytes"],
        "the canonical file is not the size the image should be",
    )

    # Every case was repeated: the user's floor is five runs.
    runs = r["parameters"]["runs"]
    check(runs >= 5, f"only {runs} runs per case; the floor is 5")
    quoted(runs, "d")

    # ---- the encode matrix ---------------------------------------------------
    by_tile: dict[int, list[dict]] = {}
    for row in s["encode_matrix"]:
        by_tile.setdefault(row["tile_size"], []).append(row)
        check(row["runs"] == runs, f"the encode case t{row['tile_size']} ran {row['runs']} times")
        stats = row["stats"]
        check(
            stats["count"] == runs,
            f"the encode case t{row['tile_size']} w{row['workers']} has {stats['count']} samples",
        )
        check(
            stats["min_ms"] <= stats["median_ms"] <= stats["max_ms"],
            f"the encode case t{row['tile_size']} w{row['workers']} has an impossible order",
        )
        check(
            stats["median_ms"] <= stats["p95_ms"] <= stats["max_ms"],
            f"the encode case t{row['tile_size']} w{row['workers']} has an impossible p95",
        )
        quoted(stats["median_ms"], ".2f")
        quoted(stats["p95_ms"], ".2f")
        quoted(stats["mean_ms"], ".2f")
        quoted(stats["stdev_ms"], ".2f")
        quoted(row["file_size_bytes"], ",")
        quoted(row["sha256"], "")

    # The worker count must not change the file: within a tile size every
    # worker count has to have written the same bytes.
    for tile_size, rows in by_tile.items():
        digests = {row["sha256"] for row in rows}
        check(
            len(digests) == 1,
            f"tile size {tile_size}: the worker counts wrote different files",
        )

    # ---- the compression summary --------------------------------------------
    for row in s["compression"]:
        quoted(row["file_size_bytes"], ",")
        quoted(row["canonical_bytes"], ",")
        quoted(row["tiles"], ",")
        quoted(row["ratio"], ".3f")
        quoted(row["sha256"], "")
        check(
            row["all_runs_identical"],
            f"tile size {row['tile_size']}: the runs did not all write the same file",
        )
        check(
            row["file_size_bytes"] > 0,
            f"tile size {row['tile_size']}: the file is empty",
        )

    # ---- the viewing matrix --------------------------------------------------
    for row in s["first_viewport_matrix"]:
        quoted(row["open_stats"]["median_ms"], ".2f")
        quoted(row["decode_stats"]["median_ms"], ".2f")
        quoted(row["time_to_pixels_stats"]["median_ms"], ".2f")
        quoted(row["time_to_pixels_stats"]["p95_ms"], ".2f")
        quoted(row["visible_tiles"], "d")

    # ---- the pan matrix ------------------------------------------------------
    for row in s["pan_matrix"]:
        quoted(row["pan_stats"]["median_ms"], ".2f")
        quoted(row["pan_stats"]["p95_ms"], ".2f")
        quoted(row["sweep_total_stats"]["median_ms"], ".2f")
        quoted(row["pans_per_run"], "d")

    # ---- the sequential baseline ---------------------------------------------
    for row in s["sequential_baseline"]:
        quoted(row["stats"]["median_ms"], ".2f")
        quoted(row["stats"]["p95_ms"], ".2f")
        quoted(row["visible_tiles"], "d")

    # The report has to say what the percentiles mean and what the memory
    # scope is.
    check("nearest-rank" in report.lower(), "the report does not define the p95 rule")
    check("median" in report.lower(), "the report does not mention the median")
    check("included" in report.lower() and "excluded" in report.lower(),
          "the report does not state the memory scope")
    check("sha256" in report.lower() or "digest" in report.lower(),
          "the report does not mention the digest")


def check_corpus() -> None:
    """Check the 750-image corpus report against the raw data."""
    report = (REPO / "docs/P5_CORPUS_REPORT.md").read_text(encoding="utf-8")
    raw = json.loads((REPO / "benchmarks/p5_corpus_results.json").read_text(encoding="utf-8"))
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = raw["summary"]
    categories = summary["categories"]
    overall = summary["overall"]
    images = raw["images"]
    parameters = raw["context"]["parameters"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the corpus report does not contain the figure {text!r}")

    # ---- the corpus is the corpus ------------------------------------------
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {image["id"] for image in images}
    check(
        manifest_ids == result_ids,
        "the manifest and the measurement file do not hold the same images",
    )
    check(
        len(images) == manifest["total_images"],
        f"the measurement file holds {len(images)} images, the manifest says {manifest['total_images']}",
    )
    quoted(len(images), ",")
    quoted(len(images), "d")

    # Every image must have been the bytes the manifest recorded.
    mismatched = [image["id"] for image in images if not image["sha256_matches_manifest"]]
    check(not mismatched, f"{len(mismatched)} images do not match their manifest digest")

    # The manifest must record a digest and a commit for every image.
    for entry in manifest["images"]:
        check(len(entry.get("sha256", "")) == 64, f"{entry['id']}: no canonical digest")
        check(len(entry.get("source_sha256", "")) == 64, f"{entry['id']}: no source digest")
        check(entry.get("category") in categories, f"{entry['id']}: unknown category")
        if entry.get("origin") == "github":
            check(
                len(entry.get("source_commit", "")) == 40,
                f"{entry['id']}: no source commit",
            )

    # ---- the per-category figures ------------------------------------------
    for name, row in categories.items():
        quoted(row["count"], "d")
        quoted(row["real"], "d")
        quoted(row["synthetic"], "d")
        quoted(row["auto"]["ratio"]["median"], ".3f")
        quoted(row["auto"]["ratio"]["p95"], ".3f")
        quoted(int(row["auto"]["bytes"]["median"]), ",")
        quoted(int(row["auto"]["bytes"]["p95"]), ",")
        quoted(row["under_10mb"]["count"], "d")
        quoted(f"{100.0 * row['under_10mb']['rate']:.1f}%", "")
        for tool in ("png", "webp", "jxl"):
            if row[tool]["measured"]:
                quoted(row[tool]["ratio"]["median"], ".3f")

    # ---- the overall figures -------------------------------------------------
    quoted(overall["auto"]["ratio"]["median"], ".3f")
    quoted(overall["auto"]["ratio"]["p95"], ".3f")
    quoted(overall["png"]["ratio"]["median"], ".3f")
    quoted(overall["webp"]["ratio"]["median"], ".3f")
    quoted(overall["jxl"]["ratio"]["median"], ".3f")
    quoted(overall["under_10mb"]["count"], "d")
    quoted(f"{100.0 * overall['under_10mb']['rate']:.1f}%", "")
    quoted(overall["total_canonical_bytes"], ",")
    quoted(overall["total_auto_bytes"], ",")

    # ---- the encoder's own property -----------------------------------------
    # Auto selection mixes codecs per tile; it must never be larger than the
    # best single-codec run on the same pixels.
    comparable = [image for image in images if image.get("auto_no_worse_than_best") is not None]
    check(comparable, "no image produced a comparable auto and forced run")
    worse = [image["id"] for image in comparable if not image["auto_no_worse_than_best"]]
    check(not worse, f"{len(worse)} images are larger under auto than under the best forced codec")

    # Every encode must have passed the container's own check.
    failed = [
        image["id"]
        for image in images
        for codec, node in image["vpng"].items()
        if node.get("check_exit", 0) != 0
    ]
    check(not failed, f"{len(failed)} encodes did not pass `vpng check`")

    # ---- the report has to state its rules -----------------------------------
    check("nearest-rank" in report.lower(), "the corpus report does not define the p95 rule")
    check("median" in report.lower(), "the corpus report does not mention the median")
    check("included" in report.lower() and "excluded" in report.lower(),
          "the corpus report does not state the memory scope")
    check("sha256" in report.lower() or "digest" in report.lower(),
          "the corpus report does not mention the digest")
    check(f"{parameters['size_ceiling_bytes']:,}" in report,
          "the corpus report does not state the 10 MB ceiling in bytes")

    # The corpus's composition must be stated, not implied: a synthetic image
    # and a photograph are not the same evidence.
    check("synthetic" in report.lower(), "the corpus report does not say what is synthetic")
    check("licence" in report.lower() or "license" in report.lower(),
          "the corpus report does not mention the licences")


def check_preview() -> None:
    """Check the Phase B preview-pyramid report against the raw data."""
    report = (REPO / "docs/P6_P7_REPORT.md").read_text(encoding="utf-8")
    raw = json.loads((REPO / "benchmarks/p6_preview_results.json").read_text(encoding="utf-8"))
    r = raw["report"]
    s = r["scenarios"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the preview report does not contain the figure {text!r}")

    # ---- the source -----------------------------------------------------------
    quoted(r["source"]["canonical_bytes"], ",")
    quoted(r["source"]["width"], ",")
    quoted(r["source"]["height"], ",")
    check(
        r["source"]["canonical_bytes"] == r["source"]["file_bytes"],
        "the canonical file is not the size the image should be",
    )

    # Every case was repeated: the floor is five runs.
    runs = r["parameters"]["runs"]
    check(runs >= 5, f"only {runs} runs per case; the floor is 5")
    quoted(runs, "d")

    # ---- the two encodes ------------------------------------------------------
    base = s["encode_base"]
    pyramid = s["encode_pyramid"]
    for row in (base, pyramid):
        stats = row["stats"]
        check(
            stats["count"] == runs,
            f"the encode case {row['label']} has {stats['count']} samples, not {runs}",
        )
        check(
            stats["min_ms"] <= stats["median_ms"] <= stats["max_ms"],
            f"the encode case {row['label']} has an impossible order",
        )
        check(
            stats["median_ms"] <= stats["p95_ms"] <= stats["max_ms"],
            f"the encode case {row['label']} has an impossible p95",
        )
        check(
            row["all_runs_identical"],
            f"the encode case {row['label']} did not write the same file every run",
        )
        quoted(stats["median_ms"], ".2f")
        quoted(stats["p95_ms"], ".2f")
        quoted(stats["mean_ms"], ".2f")
        quoted(stats["stdev_ms"], ".2f")
        quoted(row["file_size_bytes"], ",")
        quoted(row["sha256"], "")

    # The v0.1 file must be the one the P5.2 baseline wrote: the pyramid work
    # must not have moved a single byte of the default path.
    p5 = json.loads((REPO / "benchmarks/p5_bench_results.json").read_text(encoding="utf-8"))
    p5_256 = [
        row
        for row in p5["report"]["scenarios"]["compression"]
        if row["tile_size"] == 256
    ]
    check(len(p5_256) == 1, "the P5.2 baseline does not have one tile-256 row")
    if p5_256:
        check(
            p5_256[0]["sha256"] == base["sha256"],
            "the v0.1 file the preview benchmark wrote is not the P5.2 baseline file",
        )
        check(
            p5_256[0]["file_size_bytes"] == base["file_size_bytes"],
            "the v0.1 file size moved since the P5.2 baseline",
        )

    # The pyramid holds more than one level, and the flag agrees with it.
    check(pyramid["level_count"] > 1, "the pyramid file has one level")
    check(
        len(pyramid["level_tile_counts"]) == pyramid["level_count"],
        "the pyramid's level counts do not match its level count",
    )
    check(
        sum(pyramid["level_tile_counts"]) == pyramid["tile_count"],
        "the pyramid's level tile counts do not add up to its tile count",
    )
    # The previews cost bytes: the pyramid file is bigger than its base level.
    check(
        pyramid["file_size_bytes"] > pyramid["base_only_file_size_bytes"],
        "the pyramid file is not bigger than its base level alone",
    )
    quoted(pyramid["level_count"], "d")
    quoted(pyramid["tile_count"], ",")
    quoted(pyramid["base_only_file_size_bytes"], ",")

    # ---- the viewports --------------------------------------------------------
    for row in (s["first_viewport_base"], s["first_viewport_pyramid"]):
        quoted(row["time_to_pixels_stats"]["median_ms"], ".2f")
        quoted(row["time_to_pixels_stats"]["p95_ms"], ".2f")
        quoted(row["visible_tiles"], "d")

    # ---- the levels -----------------------------------------------------------
    levels = s["level_decode"]
    check(len(levels) == pyramid["level_count"], "the level rows do not match the pyramid")
    previous = None
    for row in levels:
        quoted(row["stats"]["median_ms"], ".2f")
        quoted(row["stats"]["p95_ms"], ".2f")
        quoted(row["tiles"], ",")
        check(
            row["stats"]["count"] == runs,
            f"level {row['level']} has {row['stats']['count']} samples, not {runs}",
        )
        if row["level"] == 0:
            check(not row["derived"], "level 0 is marked as derived")
        else:
            check(row["derived"], f"level {row['level']} is not marked as derived")
        if previous is not None:
            check(
                row["width"] < previous[0] and row["height"] < previous[1],
                f"level {row['level']} is not strictly smaller than the level below it",
            )
        previous = (row["width"], row["height"])

    # ---- the whole picture ----------------------------------------------------
    full = s["full_decode_base"]
    quoted(full["stats"]["median_ms"], ".2f")
    quoted(full["stats"]["p95_ms"], ".2f")

    # ---- the editing memory matrix, re-run after the partition refactor -------
    memory = json.loads((REPO / "benchmarks/p6_memory_results.json").read_text(encoding="utf-8"))
    summary = memory["summary"]
    check(summary["pass"] == 12, f"the memory matrix passed {summary['pass']} cases, not 12")
    check(summary["fail"] == 0, f"the memory matrix has {summary['fail']} failures")
    check(summary["blocked"] == 0, f"the memory matrix has {summary['blocked']} blocked cases")
    check(
        "12 PASS / 0 FAIL / 0 BLOCKED" in report,
        "the report does not state the memory matrix's 12/0/0 result",
    )
    target = memory["context"]["memory_target_bytes"]
    check(target == 64 * 1024 * 1024, "the memory target is not 64 MB")
    worst_delta = max(
        case["edit"]["memory"]["working_set_delta_bytes"] for case in memory["cases"]
    )
    check(
        worst_delta <= target,
        "the worst edit in the memory matrix is over the target",
    )
    quoted(worst_delta, ",")
    quoted(target, ",")
    # Every case rewrites exactly one tile and copies the rest.
    for case in memory["cases"]:
        commit = case["edit"]["commit"]
        check(
            commit["tiles_rewritten"] == 1,
            f"the memory case {case['case']['width']}x{case['case']['height']} "
            f"{case['case']['format']} rewrote {commit['tiles_rewritten']} tiles, not 1",
        )

    # The report has to say what the percentiles mean and what the memory scope
    # is, and it has to name the memory figures it does not claim.
    check("nearest-rank" in report.lower(), "the report does not define the p95 rule")
    check("included" in report.lower() and "excluded" in report.lower(),
          "the report does not state the memory scope")
    check("PASS" in report and "BLOCKED" in report,
          "the report does not use the gate vocabulary")


def check_adaptive() -> None:
    """Check the Phase B adaptive-tiling report against the raw data."""
    report = (REPO / "docs/P6_P7_REPORT.md").read_text(encoding="utf-8")
    raw = json.loads((REPO / "benchmarks/p7_adaptive_results.json").read_text(encoding="utf-8"))
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = raw["summary"]
    overall = summary["overall"]
    images = raw["images"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the adaptive report does not contain the figure {text!r}")

    # The corpus is the corpus: the same 750 images, the same manifest.
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {image["id"] for image in images}
    check(
        manifest_ids == result_ids,
        "the manifest and the adaptive measurement file do not hold the same images",
    )
    check(
        len(images) == manifest["total_images"],
        f"the adaptive measurement file holds {len(images)} images, the manifest says "
        f"{manifest['total_images']}",
    )
    quoted(len(images), ",")

    # Every image must have been encoded both ways and checked.
    missing = [
        image["id"]
        for image in images
        if "bytes" not in image.get("uniform", {}) or "bytes" not in image.get("adaptive", {})
    ]
    check(not missing, f"{len(missing)} images were not encoded both ways")
    not_exact = [
        image["id"]
        for image in images
        if image.get("adaptive", {}).get("exact") is not True
    ]
    check(not not_exact, f"{len(not_exact)} adaptive files are not pixel-exact")
    failed_checks = [
        image["id"]
        for image in images
        if image.get("adaptive", {}).get("check_exit", 0) != 0
    ]
    check(not failed_checks, f"{len(failed_checks)} adaptive files failed `vpng check`")

    # The denominator is the same one the P5 corpus report used.
    check(
        raw["context"]["parameters"]["denominator"] == "canonical pixel bytes",
        "the adaptive run did not use the canonical pixel bytes as its denominator",
    )

    # The headline figures.
    quoted(overall["total_uniform_bytes"], ",")
    quoted(overall["total_adaptive_bytes"], ",")
    quoted(overall["adaptive_smaller"], "d")
    quoted(overall["adaptive_larger"], "d")
    quoted(overall["adaptive_equal"], "d")
    quoted(overall["exact"], "d")

    # Every category's medians.
    for category, row in summary["categories"].items():
        for name in ("uniform", "adaptive"):
            median = row[name]["ratio"].get("median")
            if median is not None:
                quoted(median, ".3f")
        quoted(row["count"], "d")
        quoted(row["adaptive_smaller"], "d")
        quoted(row["adaptive_larger"], "d")


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

    # ---- the P5 viewing deliverable -----------------------------------------
    check_p5()

    # ---- the parallel-encoder deliverable ------------------------------------
    check_encode()

    # ---- the multi-run photographic baseline ---------------------------------
    check_bench()

    # ---- the 750-image corpus ------------------------------------------------
    check_corpus()

    # ---- Phase B: the preview pyramid and the adaptive partition ---------------
    check_preview()
    check_adaptive()

    if problems:
        print(f"{len(problems)} problem(s):")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print("all quoted figures match the raw data")
    return 0


if __name__ == "__main__":
    sys.exit(main())
