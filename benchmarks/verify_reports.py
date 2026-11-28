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


def check_codecs() -> None:
    """Check the codec-promotion report against the raw data."""
    report = (REPO / "docs/P8_CODEC_REPORT.md").read_text(encoding="utf-8")
    # The reports write a negative number with a typographic minus; the
    # figures in the raw data use the ASCII one.
    report = report.replace("−", "-")
    prod = json.loads(
        (REPO / "benchmarks/p8_codec_production_results.json").read_text(encoding="utf-8")
    )
    research = json.loads(
        (REPO / "benchmarks/p8_codec_corpus_results.json").read_text(encoding="utf-8")
    )
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = prod["summary"]
    overall = summary["overall"]
    images = prod["images"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the codec report does not contain the figure {text!r}")

    # ---- the corpus is the corpus ------------------------------------------
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {image["id"] for image in images}
    check(
        manifest_ids == result_ids,
        "the manifest and the codec measurement file do not hold the same images",
    )
    check(
        len(images) == manifest["total_images"],
        f"the codec measurement file holds {len(images)} images, the manifest says "
        f"{manifest['total_images']}",
    )
    quoted(len(images), ",")
    quoted(len(images), "d")

    # ---- every file was checked and verified --------------------------------
    for name in ("stable", "extended"):
        check_failures = [
            image["id"]
            for image in images
            if image[name]["check_exit"] != 0
        ]
        check(
            not check_failures,
            f"{len(check_failures)} {name} files failed `vpng check`",
        )
        verify_failures = [
            image["id"]
            for image in images
            if image[name]["verify_exit"] != 0
        ]
        check(
            not verify_failures,
            f"{len(verify_failures)} {name} files failed `vpng verify`",
        )
        quoted(overall[name]["check_failures"], "d")
        quoted(overall[name]["verify_failures"], "d")

    # ---- the default path did not move --------------------------------------
    check(
        overall["compared_with_p5"] == len(images),
        "not every image was compared with the P5 baseline",
    )
    check(
        overall["stable_matches_p5"] == len(images),
        f"{len(images) - overall['stable_matches_p5']} stable files differ from the P5 baseline",
    )
    quoted(overall["stable_matches_p5"], "d")
    # Every stable file is a v0.1 file.
    check(
        overall["stable"]["files_by_minor"] == {"1": len(images)},
        f"the stable run wrote {overall['stable']['files_by_minor']}, not all v0.1 files",
    )

    # ---- the headline figures -------------------------------------------------
    quoted(overall["stable"]["total_bytes"], ",")
    quoted(overall["extended"]["total_bytes"], ",")
    delta = overall["extended"]["total_bytes"] - overall["stable"]["total_bytes"]
    quoted(delta, ",")
    quoted(f"{100.0 * delta / overall['stable']['total_bytes']:.2f}%", "")
    quoted(overall["extended_smaller"], "d")
    quoted(overall["extended_larger"], "d")
    quoted(overall["extended_equal"], "d")
    quoted(overall["stable"]["ratio"]["median"], ".6f")
    quoted(overall["extended"]["ratio"]["median"], ".6f")

    # The extended selection must never be larger.
    check(
        overall["extended_larger"] == 0,
        f"{overall['extended_larger']} images are larger under the extended set",
    )
    check(
        overall["extended_smaller"] + overall["extended_equal"] == len(images),
        "the smaller/equal/larger counts do not add up",
    )

    # ---- the per-category figures --------------------------------------------
    for name, row in summary["categories"].items():
        quoted(row["count"], "d")
        quoted(row["stable"]["total_bytes"], ",")
        quoted(row["extended"]["total_bytes"], ",")
        quoted(row["stable"]["ratio"]["median"], ".6f")
        quoted(row["extended"]["ratio"]["median"], ".6f")
        quoted(row["extended_smaller"], "d")
        quoted(row["extended_larger"], "d")
        check(
            row["extended_larger"] == 0,
            f"{name}: {row['extended_larger']} images are larger under the extended set",
        )

    # ---- the version states what the file contains --------------------------
    minors = overall["extended"]["files_by_minor"]
    check(
        minors.get("3", 0) == overall["extended_smaller"],
        f"the extended run wrote {minors.get('3', 0)} v0.3 files, but "
        f"{overall['extended_smaller']} images differ from the stable set",
    )
    check(
        minors.get("1", 0) == overall["extended_equal"],
        f"the extended run wrote {minors.get('1', 0)} v0.1 files, but "
        f"{overall['extended_equal']} images are unchanged",
    )
    for image in images:
        uses_promoted = any(
            codec in ("lz77", "predictor+dictionary", "context-entropy")
            for codec in image["extended"]["header"]["tiles_by_codec"]
        )
        if uses_promoted:
            check(
                image["extended"]["header"]["minor"] == 3,
                f"{image['id']}: names a promoted codec but is not a v0.3 file",
            )
        else:
            check(
                image["extended"]["header"]["minor"] == 1,
                f"{image['id']}: names no promoted codec but is not a v0.1 file",
            )

    # ---- the encode-time cost -------------------------------------------------
    stable_ms = sum(image["stable"]["encode_ms"] for image in images)
    extended_ms = sum(image["extended"]["encode_ms"] for image in images)
    quoted(stable_ms, ",")
    quoted(extended_ms, ",")
    quoted(f"{extended_ms / stable_ms:.2f}", "")
    check(
        extended_ms > stable_ms,
        "the extended set was not slower than the stable set",
    )

    # ---- the candidates, measured on their own --------------------------------
    research_overall = research["summary"]["overall"]
    for name in ("context-entropy", "predictor+dictionary", "lz77", "predictor"):
        candidate = name
        if name == "predictor+dictionary":
            candidate = "pred-lz77"
        if name == "context-entropy":
            candidate = "carc-c"
        row = research_overall.get(candidate)
        check(row is not None, f"the research harness has no {candidate}")
        if row is None or row.get("count", 0) == 0:
            continue
        quoted(row["total_bytes"], ",")
        quoted(row["ratio"]["median"], ".6f")
        quoted(row["ratio"]["p95"], ".6f")
        quoted(row["exact"], "d")

    # The research projection and the production run must agree exactly.
    check(
        research_overall["stable_auto"]["total_bytes"] == overall["stable"]["total_bytes"],
        "the research projection and the production run disagree on the stable total",
    )
    check(
        research_overall["extended_auto"]["total_bytes"] == overall["extended"]["total_bytes"],
        "the research projection and the production run disagree on the extended total",
    )

    # ---- the determinism spot check --------------------------------------------
    check(
        overall["determinism_checked"] > 0,
        "no image was re-encoded with a different worker count",
    )
    check(
        overall["determinism_failures"] == 0,
        f"{overall['determinism_failures']} images changed with the worker count",
    )
    quoted(overall["determinism_checked"], "d")
    quoted(overall["determinism_failures"], "d")

    # ---- the 16K photograph -------------------------------------------------
    big = [image for image in images if image["canonical_bytes"] > 100_000_000]
    check(len(big) == 1, "the corpus does not hold exactly one 16K-scale image")
    if big:
        photo = big[0]
        quoted(photo["stable"]["bytes"], ",")
        quoted(photo["extended"]["bytes"], ",")
        quoted(photo["delta_bytes"], ",")
        quoted(photo["stable"]["sha256"], "")
        quoted(photo["extended"]["sha256"], "")
        # The stable file must be the P5.2 baseline's tile-256 file.
        bench = json.loads(
            (REPO / "benchmarks/p5_bench_results.json").read_text(encoding="utf-8")
        )
        rows = [
            row
            for row in bench["report"]["scenarios"]["compression"]
            if row["tile_size"] == 256
        ]
        check(len(rows) == 1, "the P5.2 baseline does not have one tile-256 row")
        if rows:
            check(
                rows[0]["sha256"] == photo["stable"]["sha256"],
                "the 16K stable file is not the P5.2 baseline file",
            )
            check(
                rows[0]["file_size_bytes"] == photo["stable"]["bytes"],
                "the 16K stable file size moved since the P5.2 baseline",
            )

    # ---- the size ceiling ------------------------------------------------------
    check(
        overall["stable"]["under_10mb"] == 748,
        f"{overall['stable']['under_10mb']} stable files are under 10 MB, not 748",
    )
    check(
        overall["extended"]["under_10mb"] == 748,
        f"{overall['extended']['under_10mb']} extended files are under 10 MB, not 748",
    )
    quoted(overall["stable"]["under_10mb"], "d")
    quoted(overall["extended"]["under_10mb"], "d")
    for image in images:
        if image["extended"]["bytes"] > 10_000_000:
            quoted(image["extended"]["bytes"], ",")

    # ---- the report has to state its rules -----------------------------------
    check("nearest-rank" in report.lower(), "the codec report does not define the p95 rule")
    check("median" in report.lower(), "the codec report does not mention the median")
    check("included" in report.lower() and "excluded" in report.lower(),
          "the codec report does not state the memory scope")
    check("sha256" in report.lower() or "digest" in report.lower(),
          "the codec report does not mention the digest")
    check("PASS" in report and "BLOCKED" in report,
          "the codec report does not use the gate vocabulary")
    check("lossless" in report.lower() or "exact" in report.lower(),
          "the codec report does not state the lossless property")


def check_python() -> None:
    """Check the Python-binding report against the raw data."""
    report = (REPO / "docs/P9_PYTHON_REPORT.md").read_text(encoding="utf-8")
    report = report.replace("−", "-")
    raw = json.loads(
        (REPO / "benchmarks/p9_python_results.json").read_text(encoding="utf-8")
    )
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = raw["summary"]
    images = raw["images"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the Python report does not contain the figure {text!r}")

    # ---- the corpus is the corpus ------------------------------------------
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {image["id"] for image in images}
    check(
        manifest_ids == result_ids,
        "the manifest and the Python measurement file do not hold the same images",
    )
    check(
        len(images) == manifest["total_images"],
        f"the Python measurement file holds {len(images)} images, the manifest says "
        f"{manifest['total_images']}",
    )
    quoted(len(images), ",")
    quoted(len(images), "d")

    # ---- the binding writes the same bytes as the command line ---------------
    check(
        summary["identical"] == len(images),
        f"{summary['identical_differing']} Python files differ from the command line's",
    )
    check(
        summary["check_failures"] == 0,
        f"{summary['check_failures']} Python-written files failed `vpng check`",
    )
    check(
        summary["verify_failures"] == 0,
        f"{summary['verify_failures']} Python-written files failed `vpng verify`",
    )
    check(
        summary["deterministic_differing"] == 0,
        f"{summary['deterministic_differing']} files changed with the worker count",
    )
    for image in images:
        check(
            image["identical"],
            f"{image['id']}: the Python file differs from the command line's",
        )
        check(
            image["check_exit"] == 0,
            f"{image['id']}: the Python-written file failed `vpng check`",
        )
        check(
            image["verify_exit"] == 0,
            f"{image['id']}: the Python-written file failed `vpng verify`",
        )
        check(
            image["deterministic"],
            f"{image['id']}: the worker count changed the bytes",
        )

    # ---- the bytes are the published baseline's ------------------------------
    check(
        summary["file_bytes_total"] == 453_767_460,
        f"the Python total is {summary['file_bytes_total']}, the published stable "
        "baseline is 453767460",
    )
    quoted(summary["file_bytes_total"], ",")
    baseline = next(
        (image for image in images if image["id"] == "user-photo-15360x8640"), None
    )
    check(baseline is not None, "the 16K photograph is not in the Python measurement")
    if baseline is not None:
        check(
            baseline["python_sha256"]
            == "d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf",
            "the Python 16K digest is not the published P5.2 baseline digest",
        )
        check(
            baseline["file_bytes"] == 124_197_035,
            f"the Python 16K file is {baseline['file_bytes']} bytes, the published "
            "baseline is 124197035",
        )
        quoted(baseline["file_bytes"], ",")
        check(
            baseline["python_sha256"] in report,
            "the Python report does not contain the 16K digest",
        )

    # ---- the timings --------------------------------------------------------
    quoted(summary["cli_ms"]["median"], ",")
    quoted(summary["python_wall_ms"]["median"], ",")
    quoted(summary["python_encode_ms"]["median"], ",")

    # ---- the report says what it does and does not claim --------------------
    check(
        "included" in report.lower() and "excluded" in report.lower(),
        "the Python report does not state the memory scope",
    )
    check(
        "PASS" in report and "BLOCKED" in report,
        "the Python report does not use the gate vocabulary",
    )
    check(
        "lossless" in report.lower() or "exact" in report.lower(),
        "the Python report does not state the lossless property",
    )
    check(
        "12" in report and "Q12" in report,
        "the Python report does not answer all twelve questions",
    )


def check_http() -> None:
    """Check the HTTP Range report against the raw data."""
    report = (REPO / "docs/P10_HTTP_REPORT.md").read_text(encoding="utf-8")
    report = report.replace("−", "-")
    raw = json.loads(
        (REPO / "benchmarks/p10_http_results.json").read_text(encoding="utf-8")
    )
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = raw["summary"]
    images = raw["images"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the HTTP report does not contain the figure {text!r}")

    # ---- the corpus is the corpus ------------------------------------------
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {image["id"] for image in images}
    check(
        manifest_ids == result_ids,
        "the manifest and the HTTP measurement file do not hold the same images",
    )
    check(
        len(images) == manifest["total_images"],
        f"the HTTP measurement file holds {len(images)} images, the manifest says "
        f"{manifest['total_images']}",
    )
    quoted(len(images), ",")
    quoted(len(images), "d")

    # ---- the HTTP read returns the same pixels as the local read -------------
    check(
        summary["identical"] == len(images),
        f"{summary['identical_differing']} HTTP reads differ from the local read",
    )
    check(
        summary["identical_file"] == len(images),
        f"{summary['identical_file_differing']} HTTP-written PNGs differ from the "
        "local ones",
    )
    for image in images:
        check(
            image["identical"],
            f"{image['id']}: the HTTP read differs from the local read",
        )
        check(
            image["identical_file"],
            f"{image['id']}: the HTTP-written PNG differs from the local one",
        )

    # ---- the bytes are the published baseline's ------------------------------
    check(
        summary["object_bytes_total"] == 453_767_460,
        f"the HTTP corpus total is {summary['object_bytes_total']}, the published "
        "stable baseline is 453767460",
    )
    quoted(summary["object_bytes_total"], ",")
    check(
        all(image["minor"] == 1 for image in images),
        "a file read over HTTP is not a minor version 1 file",
    )

    # ---- the reader fetches only what it needs -------------------------------
    baseline = next(
        (image for image in images if image["id"] == "user-photo-15360x8640"), None
    )
    check(baseline is not None, "the 16K photograph is not in the HTTP measurement")
    if baseline is not None:
        check(
            baseline["file_bytes"] == 124_197_035,
            f"the 16K object is {baseline['file_bytes']} bytes, the published "
            "baseline is 124197035",
        )
        quoted(baseline["file_bytes"], ",")
        check(
            baseline["http_bytes_read"] < baseline["file_bytes"],
            "the 16K viewport fetched the whole object",
        )
        quoted(baseline["http_bytes_read"], ",")
        quoted(baseline["http_whole_object_fraction"] * 100, ".4f")
        quoted(baseline["info_bytes_read"], ",")
        quoted(baseline["info_whole_object_fraction"] * 100, ".4f")

    # ---- the client and the server agree on the byte count -------------------
    source_total = summary["http_source_bytes_total"] + summary["info_source_bytes_total"]
    check(
        source_total == summary["server_bytes_sent"],
        f"the source received {source_total} bytes, the server sent "
        f"{summary['server_bytes_sent']}",
    )
    quoted(summary["server_bytes_sent"], ",")
    quoted(summary["server_requests"], ",")
    quoted(summary["server_range_requests"], ",")

    # ---- the corpus-level fractions ------------------------------------------
    check(
        summary["info_fraction"] is not None and summary["info_fraction"] < 0.001,
        f"the header-and-directory share is {summary['info_fraction']}, not under "
        "0.1%",
    )
    quoted(summary["info_fraction"] * 100, ".4f")
    quoted(summary["http_fraction"] * 100, ".3f")
    quoted(summary["info_bytes_total"], ",")
    quoted(summary["http_bytes_total"], ",")

    # ---- the subset that is larger than the viewport -------------------------
    bigger = [
        image for image in images if image["width"] > 1024 or image["height"] > 1024
    ]
    check(
        len(bigger) == 158,
        f"{len(bigger)} images are larger than the viewport, the report says 158",
    )
    quoted(len(bigger), "d")
    bigger_object = sum(image["file_bytes"] for image in bigger)
    bigger_region = sum(image["http_bytes_read"] for image in bigger)
    quoted(bigger_object, ",")
    quoted(bigger_region, ",")
    quoted(bigger_region / bigger_object * 100, ".4f")

    # ---- the per-category table ----------------------------------------------
    for category, bucket in sorted(summary["by_category"].items()):
        quoted(bucket["images"], "d")
        quoted(bucket["object_bytes"], ",")
        quoted(bucket["http_bytes"], ",")
        quoted(bucket["http_fraction"] * 100, ".4f")

    # ---- the timings ---------------------------------------------------------
    quoted(summary["http_ms"]["median"], ",")
    quoted(summary["http_ms"]["p95"], ",")
    quoted(summary["local_ms"]["median"], ",")
    quoted(summary["local_ms"]["p95"], ",")

    # ---- the report says what it does and does not claim --------------------
    check(
        "included" in report.lower() and "excluded" in report.lower(),
        "the HTTP report does not state the byte-count scope",
    )
    check(
        "PASS" in report and "BLOCKED" in report,
        "the HTTP report does not use the gate vocabulary",
    )
    check(
        "lossless" in report.lower() or "exact" in report.lower(),
        "the HTTP report does not state the lossless property",
    )
    check(
        "12" in report and "Q12" in report,
        "the HTTP report does not answer all twelve questions",
    )
    check(
        "Range" in report and "206" in report and "Content-Range" in report,
        "the HTTP report does not state the range rules",
    )


def check_wasm() -> None:
    """Check the WebAssembly report against the raw data."""
    report = (REPO / "docs/P11_WASM_REPORT.md").read_text(encoding="utf-8")
    report = report.replace("−", "-")
    raw = json.loads(
        (REPO / "benchmarks/p11_wasm_results.json").read_text(encoding="utf-8")
    )
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = raw["summary"]
    images = raw["images"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the WebAssembly report does not contain the figure {text!r}")

    # ---- the corpus is the corpus ---------------------------------------------
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {image["id"] for image in images}
    check(
        manifest_ids == result_ids,
        "the manifest and the WebAssembly measurement file do not hold the same images",
    )
    check(
        len(images) == manifest["total_images"],
        f"the WebAssembly measurement file holds {len(images)} images, the manifest says "
        f"{manifest['total_images']}",
    )
    quoted(len(images), ",")
    quoted(len(images), "d")

    # ---- the module's pixels are the native pixels ----------------------------
    check(
        summary["identical"] == len(images),
        f"{summary['identical_differing']} WebAssembly reads differ from the native read",
    )
    check(
        summary["range_identical"] == len(images),
        f"{summary['range_identical_differing']} WebAssembly range reads differ from the "
        "native read",
    )
    for image in images:
        check(
            image["whole_identical"],
            f"{image['id']}: the WebAssembly read differs from the native read",
        )
        check(
            image["range_identical"],
            f"{image['id']}: the WebAssembly range read differs from the native read",
        )
        check(
            image["native_sha256"] == image["wasm_whole_sha256"] == image["wasm_range_sha256"],
            f"{image['id']}: the three digests are not the same",
        )

    # ---- the bytes are the published baseline's -------------------------------
    check(
        summary["object_bytes_total"] == 453_767_460,
        f"the WebAssembly corpus total is {summary['object_bytes_total']}, the published "
        "stable baseline is 453767460",
    )
    quoted(summary["object_bytes_total"], ",")

    # ---- the 16K photograph ---------------------------------------------------
    baseline = next(
        (image for image in images if image["id"] == "user-photo-15360x8640"), None
    )
    check(baseline is not None, "the 16K photograph is not in the WebAssembly measurement")
    if baseline is not None:
        check(
            baseline["file_bytes"] == 124_197_035,
            f"the 16K file is {baseline['file_bytes']} bytes, the published baseline is "
            "124197035",
        )
        quoted(baseline["file_bytes"], ",")
        check(
            baseline["wasm_range_bytes"] < baseline["file_bytes"],
            "the 16K viewport fetched the whole file",
        )
        quoted(baseline["wasm_range_bytes"], ",")
        quoted(baseline["range_fraction"] * 100, ".4f")
        quoted(baseline["wasm_range_requests"], "d")
        quoted(baseline["wasm_range_payload_bytes"], ",")

    # ---- the range path fetches what the viewport needs -----------------------
    quoted(summary["range_bytes_total"], ",")
    quoted(summary["range_fraction"] * 100, ".3f")
    quoted(summary["native_bytes_read_total"], ",")

    # ---- the timings ----------------------------------------------------------
    quoted(summary["wasm_whole_ms"]["median"], ",")
    quoted(summary["wasm_range_ms"]["median"], ",")
    quoted(summary["native_memory_ms"]["median"], ",")
    quoted(summary["native_file_ms"]["median"], ",")

    # ---- the per-category table -----------------------------------------------
    for category, bucket in sorted(summary["by_category"].items()):
        quoted(bucket["images"], "d")
        quoted(bucket["object_bytes"], ",")
        quoted(bucket["range_bytes"], ",")
        quoted(bucket["range_fraction"] * 100, ".4f")

    # ---- the report says what it does and does not claim ----------------------
    check(
        "included" in report.lower() and "excluded" in report.lower(),
        "the WebAssembly report does not state the measurement scope",
    )
    check(
        "PASS" in report and "BLOCKED" in report,
        "the WebAssembly report does not use the gate vocabulary",
    )
    check(
        "lossless" in report.lower() or "exact" in report.lower(),
        "the WebAssembly report does not state the lossless property",
    )
    check(
        "12" in report and "Q12" in report,
        "the WebAssembly report does not answer all twelve questions",
    )
    check(
        "wasm-bindgen" in report,
        "the WebAssembly report does not state the ABI choice",
    )


def check_layers() -> None:
    """Check the layers-and-masks report against the raw data."""
    report = (REPO / "docs/P12_LAYERS_REPORT.md").read_text(encoding="utf-8")
    report = report.replace("−", "-")
    raw = json.loads(
        (REPO / "benchmarks/p12_layers_results.json").read_text(encoding="utf-8")
    )
    memory = json.loads(
        (REPO / "benchmarks/p12_layers_memory_results.json").read_text(encoding="utf-8")
    )
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = raw["summary"]
    overall = summary["overall"]
    records = raw["records"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the layers report does not contain the figure {text!r}")

    # ---- the corpus is the corpus ---------------------------------------------
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {record["id"] for record in records}
    check(
        manifest_ids == result_ids,
        "the manifest and the layers measurement file do not hold the same images",
    )
    check(
        len(records) == manifest["total_images"],
        f"the layers measurement file holds {len(records)} images, the manifest says "
        f"{manifest['total_images']}",
    )
    quoted(len(records), ",")
    quoted(len(records), "d")

    # ---- the plain files are the published baseline's -------------------------
    check(
        overall["plain_bytes"] == 453_767_460,
        f"the plain total is {overall['plain_bytes']}, the published stable baseline is "
        "453767460",
    )
    quoted(overall["plain_bytes"], ",")
    check(
        overall["canonical_bytes"] == 1_450_363_330,
        f"the canonical total is {overall['canonical_bytes']}, the published one is "
        "1450363330",
    )
    quoted(overall["canonical_bytes"], ",")

    # ---- the headline counts ---------------------------------------------------
    check(
        overall["one_composite_exact"] == len(records),
        f"{len(records) - overall['one_composite_exact']} one-layer composites are not "
        "exactly the base's pixels",
    )
    check(
        overall["two_base_exact"] == len(records),
        f"{len(records) - overall['two_base_exact']} base layers are not exactly the "
        "original's pixels",
    )
    check(
        overall["two_composite_differs"] == len(records),
        f"{len(records) - overall['two_composite_differs']} two-layer composites do not "
        "differ from the base",
    )
    check(overall["check_failures"] == 0, f"{overall['check_failures']} files failed the check")
    quoted(overall["one_composite_exact"], "d")
    quoted(overall["two_base_exact"], "d")
    quoted(overall["two_composite_differs"], "d")
    quoted(overall["check_failures"], "d")

    # ---- the masks -------------------------------------------------------------
    check(
        overall["masked_images"] == 114,
        f"{overall['masked_images']} images were given a mask, the report says 114",
    )
    check(
        overall["masked_check_failures"] == 0,
        f"{overall['masked_check_failures']} masked files failed the check",
    )
    check(
        overall["masked_composite_differs"] == overall["masked_images"],
        f"{overall['masked_composite_differs']} masked composites differ from the "
        f"unmasked ones, not {overall['masked_images']}",
    )
    quoted(overall["masked_images"], "d")
    quoted(overall["masked_check_failures"], "d")
    quoted(overall["masked_composite_differs"], "d")

    # Every image whose format has an alpha channel must have been masked.
    alpha_formats = {"rgba8", "graya8"}
    expected_masked = sum(
        1 for record in records if record["pixel_format"] in alpha_formats
    )
    check(
        expected_masked == overall["masked_images"],
        f"{expected_masked} images have an alpha format, but {overall['masked_images']} "
        "were masked",
    )

    # ---- the layer table's cost -------------------------------------------------
    overheads = {record["one_bytes"] - record["plain_bytes"] for record in records}
    check(
        overheads == {60},
        f"the layer table's cost is not 60 bytes on every file: {sorted(overheads)}",
    )
    check(
        overall["one_overhead_bytes"] == 45_000,
        f"the layer table's cost over the corpus is {overall['one_overhead_bytes']}, "
        "not 45000",
    )
    quoted(overall["one_bytes"], ",")
    quoted(overall["one_overhead_bytes"], ",")
    quoted(overall["one_overhead_ratio"] * 100, ".6f")

    # The worst per-file share: the smallest file in the corpus.
    worst = max(
        records,
        key=lambda record: (record["one_bytes"] - record["plain_bytes"])
        / record["plain_bytes"],
    )
    worst_share = (
        (worst["one_bytes"] - worst["plain_bytes"]) / worst["plain_bytes"] * 100
    )
    check(
        worst["plain_bytes"] == 111,
        f"the worst overhead share belongs to a {worst['plain_bytes']}-byte file, not "
        "a 111-byte one",
    )
    quoted(worst["plain_bytes"], "d")
    quoted(worst_share, ".6f")

    # The one-layer file must be bigger than the plain file and smaller than the
    # two-layer file.
    check(
        overall["plain_bytes"] < overall["one_bytes"] < overall["two_bytes"],
        "the three file sizes are not in order",
    )
    quoted(overall["two_bytes"], ",")
    quoted(overall["two_overhead_bytes"], ",")

    # ---- the layers are encoded independently ------------------------------------
    for record in records:
        base_one = record["one_layers"][0]
        base_two = record["two_layers"][0]
        check(
            base_one["compressed_bytes"] == base_two["compressed_bytes"],
            f"{record['id']}: the base layer's compressed bytes changed when the overlay "
            "was added",
        )
        check(
            base_one["uncompressed_bytes"] == base_two["uncompressed_bytes"],
            f"{record['id']}: the base layer's uncompressed bytes changed when the "
            "overlay was added",
        )
        check(
            base_two["uncompressed_bytes"] == record["canonical_bytes"],
            f"{record['id']}: the base layer's uncompressed bytes are not the canonical "
            "bytes",
        )

    # ---- the per-category table -------------------------------------------------
    for name, row in summary["by_category"].items():
        quoted(row["count"], "d")
        quoted(row["plain_bytes"], ",")
        quoted(row["one_bytes"], ",")
        quoted(row["one_overhead_bytes"], ",")
        quoted(row["one_overhead_ratio"] * 100, ".6f")
        check(
            row["one_composite_exact"] == row["count"],
            f"{name}: not every one-layer composite is exact",
        )
        check(
            row["two_base_exact"] == row["count"],
            f"{name}: not every base layer is exact",
        )
        check(
            row["check_failures"] == 0,
            f"{name}: {row['check_failures']} files failed the check",
        )

    # The category totals must add up to the overall total.
    check(
        sum(row["plain_bytes"] for row in summary["by_category"].values())
        == overall["plain_bytes"],
        "the per-category plain totals do not add up to the overall total",
    )
    check(
        sum(row["one_bytes"] for row in summary["by_category"].values())
        == overall["one_bytes"],
        "the per-category one-layer totals do not add up to the overall total",
    )

    # ---- the 16K photograph ------------------------------------------------------
    baseline = next(
        (record for record in records if record["id"] == "user-photo-15360x8640"), None
    )
    check(baseline is not None, "the 16K photograph is not in the layers measurement")
    if baseline is not None:
        check(
            baseline["plain_bytes"] == 124_197_035,
            f"the 16K plain file is {baseline['plain_bytes']} bytes, the published "
            "baseline is 124197035",
        )
        check(
            baseline["plain_sha256"]
            == "d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf",
            "the 16K plain digest is not the published P5.2 baseline digest",
        )
        check(
            baseline["one_bytes"] == baseline["plain_bytes"] + 60,
            "the 16K one-layer file is not the plain file plus 60 bytes",
        )
        quoted(baseline["plain_bytes"], ",")
        quoted(baseline["one_bytes"], ",")
        quoted(baseline["two_bytes"], ",")
        quoted(baseline["plain_sha256"], "")
        quoted(baseline["one_sha256"], "")
        for layer in baseline["two_layers"]:
            if layer["index"] == 0:
                quoted(layer["ratio"], ".6f")
            else:
                quoted(layer["ratio"], ".6f")

    # ---- the ratio distribution --------------------------------------------------
    quoted(summary["layer_ratio_distribution"]["median"], ".6f")
    quoted(summary["layer_ratio_distribution"]["p95"], ".6f")
    quoted(summary["layer_ratio_distribution"]["mean"], ".6f")
    quoted(summary["layer_ratio_distribution"]["stdev"], ".6f")
    quoted(summary["overlay_ratio_distribution"]["median"], ".6f")
    quoted(summary["overlay_ratio_distribution"]["p95"], ".6f")
    quoted(summary["overlay_ratio_distribution"]["mean"], ".6f")
    quoted(summary["overlay_ratio_distribution"]["stdev"], ".6f")
    check(
        summary["layer_ratio_distribution"]["count"] == 2 * len(records),
        "the ratio distribution does not hold every layer",
    )
    check(
        summary["layer_ratio_distribution"]["min"] == 1.0,
        "the smallest layer ratio is not 1.0",
    )

    # ---- the memory figures ------------------------------------------------------
    for name, case in memory["cases"].items():
        quoted(case["peak_working_set_bytes"], ",")
        check(
            case["exit"] == 0,
            f"the memory case {name} exited with {case['exit']}",
        )
    check(
        memory["image"]["canonical_bytes"] == 398_131_200,
        f"the memory case's image is {memory['image']['canonical_bytes']} canonical "
        "bytes, not 398131200",
    )
    quoted(memory["image"]["canonical_bytes"], ",")
    quoted(memory["files"]["one_layer_bytes"], ",")
    quoted(memory["files"]["two_layer_bytes"], ",")
    check(
        "included" in memory["scope"] and "excluded" in memory["scope"],
        "the memory scope does not say what is included and what is excluded",
    )

    # ---- the size ceiling --------------------------------------------------------
    check(
        overall["one_under_ceiling"] == 748,
        f"{overall['one_under_ceiling']} one-layer files are under 10 MB, not 748",
    )
    check(
        overall["plain_under_ceiling"] == overall["one_under_ceiling"],
        "the layer table pushed a file over the 10 MB ceiling",
    )
    quoted(overall["one_under_ceiling"], "d")

    # ---- the report says what it does and does not claim -------------------------
    check(
        "included" in report.lower() and "excluded" in report.lower(),
        "the layers report does not state the measurement scope",
    )
    check(
        "PASS" in report and "BLOCKED" in report,
        "the layers report does not use the gate vocabulary",
    )
    check(
        "lossless" in report.lower() or "exact" in report.lower(),
        "the layers report does not state the lossless property",
    )
    check(
        "12" in report and "Q12" in report,
        "the layers report does not answer all twelve questions",
    )
    check(
        "refuses" in report.lower(),
        "the layers report does not say that the editing engine refuses a layered file",
    )


def check_gpu() -> None:
    """Check the GPU-payload report against the raw data."""
    report = (REPO / "docs/P13_GPU_REPORT.md").read_text(encoding="utf-8")
    report = report.replace("−", "-")
    raw = json.loads(
        (REPO / "benchmarks/p13_gpu_results.json").read_text(encoding="utf-8")
    )
    memory = json.loads(
        (REPO / "benchmarks/p13_gpu_memory_results.json").read_text(encoding="utf-8")
    )
    manifest = json.loads((REPO / "benchmarks/corpus_manifest.json").read_text(encoding="utf-8"))
    summary = raw["summary"]
    overall = summary["overall"]
    records = raw["records"]

    def quoted(value, fmt) -> None:
        text = format(value, fmt)
        check(text in report, f"the GPU report does not contain the figure {text!r}")

    # ---- the corpus is the corpus ---------------------------------------------
    manifest_ids = {entry["id"] for entry in manifest["images"]}
    result_ids = {record["id"] for record in records}
    check(
        manifest_ids == result_ids,
        "the manifest and the GPU measurement file do not hold the same images",
    )
    check(
        len(records) == manifest["total_images"],
        f"the GPU measurement file holds {len(records)} images, the manifest says "
        f"{manifest['total_images']}",
    )
    quoted(len(records), ",")
    quoted(len(records), "d")

    # ---- the plain files are the published baseline's -------------------------
    check(
        overall["plain_bytes"] == 453_767_460,
        f"the plain total is {overall['plain_bytes']}, the published stable baseline is "
        "453767460",
    )
    quoted(overall["plain_bytes"], ",")
    check(
        overall["canonical_bytes"] == 1_450_363_330,
        f"the canonical total is {overall['canonical_bytes']}, the published one is "
        "1450363330",
    )
    quoted(overall["canonical_bytes"], ",")

    # ---- the headline counts ---------------------------------------------------
    check(
        overall["lossless_exact"] == len(records),
        f"{len(records) - overall['lossless_exact']} GPU files do not reconstruct the "
        "picture exactly",
    )
    check(overall["check_failures"] == 0, f"{overall['check_failures']} files failed the check")
    check(
        overall["bytes_add_up_failures"] == 0,
        f"{overall['bytes_add_up_failures']} files do not add up",
    )
    check(
        overall["minor_versions"] == [5],
        f"the GPU files report minor versions {overall['minor_versions']}, not [5]",
    )
    quoted(overall["lossless_exact"], "d")
    quoted(overall["check_failures"], "d")
    quoted(overall["bytes_add_up_failures"], "d")

    # The GPU payload is lossy — but a picture BC1 happens to represent
    # exactly comes back identical, and that is measured rather than assumed.
    check(
        overall["gpu_payload_differs"] == 746,
        f"{overall['gpu_payload_differs']} GPU payloads differ from the original, not 746",
    )
    identical = len(records) - overall["gpu_payload_differs"]
    check(
        identical == 4,
        f"{identical} GPU payloads are identical to the original, not 4",
    )
    quoted(overall["gpu_payload_differs"], "d")
    quoted(identical, "d")
    for record in records:
        if record["fidelity"]["max_absolute_error"] == 0:
            check(
                record["fidelity"]["psnr_db"] is None,
                f"{record['id']}: an identical payload has a PSNR",
            )

    # ---- the GPU payload's cost -------------------------------------------------
    quoted(overall["gpu_file_bytes"], ",")
    quoted(overall["gpu_payload_bytes"], ",")
    quoted(overall["gpu_table_bytes"], ",")
    quoted(overall["gpu_overhead_bytes"], ",")
    quoted(overall["gpu_overhead_ratio"] * 100, ".6f")
    quoted(overall["gpu_payload_ratio_to_canonical"] * 100, ".6f")

    # The file must be the plain file plus the GPU table, the GPU payloads and
    # the eight bytes the header grew by.
    for record in records:
        check(
            record["bytes_add_up"],
            f"{record['id']}: the file does not add up",
        )
        check(
            record["gpu_file_bytes"] > record["plain_bytes"],
            f"{record['id']}: the GPU file is not bigger than the plain one",
        )
        check(
            record["gpu_minor"] == 5,
            f"{record['id']}: the GPU file is minor version {record['gpu_minor']}",
        )
        check(
            record["gpu_record_count"] == record["gpu_tile_count"],
            f"{record['id']}: the GPU table has {record['gpu_record_count']} records for "
            f"{record['gpu_tile_count']} tiles",
        )

    # ---- the per-category table -------------------------------------------------
    for name, row in summary["by_category"].items():
        quoted(row["count"], "d")
        quoted(row["plain_bytes"], ",")
        quoted(row["gpu_file_bytes"], ",")
        quoted(row["gpu_payload_bytes"], ",")
        quoted(row["gpu_table_bytes"], ",")
        quoted(row["gpu_overhead_bytes"], ",")
        quoted(row["gpu_overhead_ratio"] * 100, ".6f")
        quoted(row["gpu_payload_ratio_to_canonical"] * 100, ".6f")
        quoted(row["fidelity"]["psnr_db"], ".4f")
        check(
            row["lossless_exact"] == row["count"],
            f"{name}: not every GPU file reconstructs the picture exactly",
        )
        check(row["check_failures"] == 0, f"{name}: {row['check_failures']} files failed")
        check(
            row["bytes_add_up_failures"] == 0,
            f"{name}: {row['bytes_add_up_failures']} files do not add up",
        )

    # The category totals must add up to the overall total.
    check(
        sum(row["plain_bytes"] for row in summary["by_category"].values())
        == overall["plain_bytes"],
        "the per-category plain totals do not add up to the overall total",
    )
    check(
        sum(row["gpu_file_bytes"] for row in summary["by_category"].values())
        == overall["gpu_file_bytes"],
        "the per-category GPU totals do not add up to the overall total",
    )

    # ---- the fidelity ------------------------------------------------------------
    quoted(overall["fidelity"]["psnr_db"], ".4f")
    quoted(overall["fidelity"]["mean_absolute_error"], ".6f")
    quoted(overall["fidelity"]["max_absolute_error"], "d")
    quoted(overall["psnr_distribution"]["median"], ".4f")
    quoted(overall["psnr_distribution"]["min"], ".4f")
    quoted(overall["psnr_distribution"]["max"], ".4f")
    quoted(overall["max_error_distribution"]["median"], ".0f")
    quoted(overall["max_error_distribution"]["max"], ".0f")

    # ---- the encode time ----------------------------------------------------------
    quoted(summary["encode_ms"]["plain"]["median"], ".0f")
    quoted(summary["encode_ms"]["gpu"]["median"], ".0f")
    quoted(summary["encode_ms"]["gpu_over_plain"], ".4f")

    # ---- the 16K photograph ---------------------------------------------------------
    baseline = next(
        (record for record in records if record["id"] == "user-photo-15360x8640"), None
    )
    check(baseline is not None, "the 16K photograph is not in the GPU measurement")
    if baseline is not None:
        check(
            baseline["plain_bytes"] == 124_197_035,
            f"the 16K plain file is {baseline['plain_bytes']} bytes, the published "
            "baseline is 124197035",
        )
        check(
            baseline["plain_sha256"]
            == "d029a7ebc1e0347cb3399ab6f9de9653da5853c96b38d95cddc3b8bb45a14dbf",
            "the 16K plain digest is not the published P5.2 baseline digest",
        )
        quoted(baseline["plain_bytes"], ",")
        quoted(baseline["gpu_file_bytes"], ",")
        quoted(baseline["gpu_payload_bytes"], ",")
        quoted(baseline["plain_sha256"], "")
        quoted(baseline["gpu_file_sha256"], "")
        quoted(baseline["fidelity"]["psnr_db"], ".4f")

    # ---- the memory figures ----------------------------------------------------------
    check(
        memory["image"]["canonical_bytes"] == 398_131_200,
        f"the memory case's image is {memory['image']['canonical_bytes']} canonical "
        "bytes, not 398131200",
    )
    quoted(memory["image"]["canonical_bytes"], ",")
    quoted(memory["files"]["plain_bytes"], ",")
    quoted(memory["files"]["gpu_bytes"], ",")
    quoted(memory["files"]["gpu_payload_bytes"], ",")
    quoted(memory["files"]["gpu_table_bytes"], ",")
    quoted(memory["files"]["gpu_overhead_bytes"], ",")

    # The file must be the plain file plus the GPU table, the GPU payloads
    # and the eight bytes the header grew by.
    check(
        memory["files"]["gpu_overhead_bytes"]
        == memory["files"]["gpu_payload_bytes"]
        + memory["files"]["gpu_table_bytes"]
        + 8,
        "the memory case's file does not add up",
    )
    check(
        memory["files"]["gpu_payload_bytes"]
        == baseline["gpu_payload_bytes"]
        if baseline is not None
        else False,
        "the memory case's GPU payload is not the corpus measurement's",
    )
    for name, case in memory["cases"].items():
        quoted(case["peak_working_set_bytes"], ",")
        check(
            case["exit"] == 0,
            f"the memory case {name} exited with {case['exit']}",
        )
    check(
        "included" in memory["scope"] and "excluded" in memory["scope"],
        "the memory scope does not say what is included and what is excluded",
    )

    # ---- the size ceiling --------------------------------------------------------------
    check(
        overall["gpu_under_ceiling"] <= overall["plain_under_ceiling"],
        "the GPU payload pushed a file over the 10 MB ceiling that was under it",
    )
    quoted(overall["plain_under_ceiling"], "d")
    quoted(overall["gpu_under_ceiling"], "d")

    # ---- the report says what it does and does not claim ---------------------------------
    check(
        "included" in report.lower() and "excluded" in report.lower(),
        "the GPU report does not state the measurement scope",
    )
    check(
        "PASS" in report and "BLOCKED" in report,
        "the GPU report does not use the gate vocabulary",
    )
    check(
        "lossy" in report.lower(),
        "the GPU report does not state that the GPU payload is lossy",
    )
    check(
        "12" in report and "Q12" in report,
        "the GPU report does not answer all twelve questions",
    )
    check(
        "refuses" in report.lower(),
        "the GPU report does not say that the editing engine refuses a GPU file",
    )
    check(
        "BC1" in report and "BC4" in report,
        "the GPU report does not name the block codecs it measures",
    )


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

    # ---- Step 5: the promoted codecs ------------------------------------------
    check_codecs()

    # ---- Phase C: the Python bindings ----------------------------------------
    check_python()

    # ---- Phase C: the HTTP Range reader --------------------------------------
    check_http()

    # ---- Phase C: the WebAssembly package -------------------------------------
    check_wasm()

    # ---- Phase C: layers and masks ------------------------------------------------
    check_layers()

    # ---- Phase C: the GPU payload ---------------------------------------------------
    check_gpu()

    if problems:
        print(f"{len(problems)} problem(s):")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print("all quoted figures match the raw data")
    return 0


if __name__ == "__main__":
    sys.exit(main())
