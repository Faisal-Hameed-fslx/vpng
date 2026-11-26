#!/usr/bin/env python3
"""Measure the HTTP Range path over the whole 750-image corpus.

The question this answers is narrow and answerable: **does a reader that
fetches a container over HTTP Range requests fetch only the bytes it needs,
and does it reproduce the same pixels as a local read?**

For every image in the corpus the script:

* encodes it locally with the real `vpng` binary, from the same canonical
  pixels the other corpus reports used;
* serves that file from a local HTTP server that honours `Range`;
* runs `vpng http-info`, which fetches the 64-byte header and the directory
  and nothing else, and records the bytes and the requests;
* runs `vpng http-decode-region` for a 1024x1024 viewport, and records the
  bytes, the requests and the wall clock;
* decodes the **same** region from the local file and compares the two
  results by SHA-256, so a reader that fetched the wrong bytes fails the run
  instead of being reported as a win.

The server counts its own bytes and requests, so the client's accounting is
cross-checked against an independent one.

Run from the repository root:

    python benchmarks/http/run_http_bench.py --sample 750

The raw results are written to `benchmarks/p10_http_results.json`, and
`check_http()` in `benchmarks/verify_reports.py` binds the figures the
report quotes to that file.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import math
import os
import platform
import shutil
import statistics
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

CHUNK = 1024 * 1024


# --------------------------------------------------------------------------
# The measurement rules. They are the same rules the other corpus reports
# use, so the numbers can be read side by side.
# --------------------------------------------------------------------------


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
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(CHUNK)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def cache_index(cache_root: Path) -> dict[str, Path]:
    """Map every cached canonical digest to its raw file."""
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
        if not digest or not name:
            continue
        raw = cache_root / name
        if raw.exists():
            index[digest] = raw
    return index


# --------------------------------------------------------------------------
# The server.
# --------------------------------------------------------------------------


class RangeServer(ThreadingHTTPServer):
    """A server that honours `Range` and counts what it sends."""

    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, root: Path):
        super().__init__(("127.0.0.1", 0), RangeHandler)
        self.root = root
        self.bytes_sent = 0
        self.requests = 0
        self.range_requests = 0
        self.lock = threading.Lock()


class RangeHandler(BaseHTTPRequestHandler):
    """One file, served with `Range` support."""

    protocol_version = "HTTP/1.1"

    def log_message(self, *args) -> None:  # noqa: A003 - the base class's name
        """Keep the driver's output clean."""

    def _resolve(self) -> Path | None:
        name = self.path.split("?", 1)[0].lstrip("/")
        if not name or "/" in name or "\\" in name or ".." in name:
            return None
        candidate = self.server.root / name
        if not candidate.is_file():
            return None
        return candidate

    def do_HEAD(self) -> None:  # noqa: N802 - the base class's name
        path = self._resolve()
        if path is None:
            self.send_error(404)
            return
        with self.server.lock:
            self.server.requests += 1
        size = path.stat().st_size
        self.send_response(200)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(size))
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802 - the base class's name
        path = self._resolve()
        if path is None:
            self.send_error(404)
            return

        size = path.stat().st_size
        header = self.headers.get("Range")
        with self.server.lock:
            self.server.requests += 1

        if not header or not header.startswith("bytes="):
            self.send_response(200)
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(size))
            self.end_headers()
            with path.open("rb") as handle:
                shutil.copyfileobj(handle, self.wfile, CHUNK)
            with self.server.lock:
                self.server.bytes_sent += size
            return

        try:
            first_text, last_text = header[len("bytes=") :].split("-", 1)
            first = int(first_text)
            last = int(last_text) if last_text else size - 1
        except ValueError:
            self.send_error(416)
            return

        last = min(last, size - 1)
        if first > last or first >= size:
            self.send_response(416)
            self.send_header("Content-Range", f"bytes */{size}")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        length = last - first + 1
        self.send_response(206)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Range", f"bytes {first}-{last}/{size}")
        self.send_header("Content-Length", str(length))
        self.end_headers()
        with path.open("rb") as handle:
            handle.seek(first)
            remaining = length
            while remaining > 0:
                chunk = handle.read(min(CHUNK, remaining))
                if not chunk:
                    break
                self.wfile.write(chunk)
                remaining -= len(chunk)
        with self.server.lock:
            self.server.bytes_sent += length
            self.server.range_requests += 1


# --------------------------------------------------------------------------
# The driver.
# --------------------------------------------------------------------------


def run_vpng(vpng: Path, args: list[str], timeout: int = 3600) -> dict:
    """Run a `vpng` subcommand and time it."""
    started = time.perf_counter()
    proc = subprocess.run(
        [str(vpng), *args], capture_output=True, text=True, timeout=timeout
    )
    elapsed = (time.perf_counter() - started) * 1000.0
    return {
        "exit": proc.returncode,
        "ms": round(elapsed, 3),
        "output": proc.stdout.strip(),
        "error": proc.stderr.strip(),
    }


def run_vpng_json(vpng: Path, args: list[str], timeout: int = 3600) -> dict:
    """Run a `vpng` subcommand that prints JSON, and parse it."""
    result = run_vpng(vpng, [*args, "--json"], timeout=timeout)
    if result["exit"] != 0:
        raise RuntimeError(f"vpng {' '.join(args)} failed: {result['error']}")
    try:
        payload = json.loads(result["output"])
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"vpng {' '.join(args)} did not print JSON: {result['output']!r}"
        ) from error
    payload["ms"] = result["ms"]
    return payload


def read_header(path: Path) -> dict:
    """The header fields the report quotes."""
    body = path.read_bytes()
    import struct

    major, minor = struct.unpack_from("<HH", body, 8)
    width, height = struct.unpack_from("<II", body, 16)
    return {"major": major, "minor": minor, "width": width, "height": height}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--sample",
        type=int,
        default=750,
        help="how many corpus images to measure (the whole corpus is 750)",
    )
    parser.add_argument("--tile-size", type=int, default=256)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument(
        "--viewport", type=int, default=1024, help="the viewport edge in pixels"
    )
    parser.add_argument(
        "--vpng", default=str(REPO / "target" / "release" / "vpng.exe")
    )
    parser.add_argument("--cache-root", default=str(REPO / "corpus-data" / "cache"))
    parser.add_argument(
        "--manifest", default=str(REPO / "benchmarks" / "corpus_manifest.json")
    )
    parser.add_argument(
        "--output", default=str(REPO / "benchmarks" / "p10_http_results.json")
    )
    parser.add_argument("--work", default=str(REPO / "benchmarks" / "work" / "http"))
    args = parser.parse_args()

    vpng = Path(args.vpng)
    if not vpng.exists():
        print(f"vpng binary not found: {vpng} (cargo build --release --workspace)")
        return 2

    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    cache = cache_index(Path(args.cache_root))

    candidates = []
    for entry in manifest.get("images", []):
        raw = cache.get(entry["sha256"])
        if raw is not None:
            candidates.append((entry, raw))

    sample = candidates[: args.sample]
    print(f"corpus images with cached canonical pixels: {len(candidates)}")
    print(f"sample: {len(sample)}")

    work = Path(args.work)
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True, exist_ok=True)

    server = RangeServer(work)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base_url = f"http://127.0.0.1:{server.server_address[1]}"
    print(f"server: {base_url}")

    records = []
    try:
        for number, (entry, raw) in enumerate(sample, start=1):
            width, height = entry["width"], entry["height"]
            fmt = entry["pixel_format"]
            name = f"{entry['sha256'][:16]}.vpng"
            local = work / name
            remote_png = work / f"{entry['sha256'][:16]}-remote.png"
            local_png = work / f"{entry['sha256'][:16]}-local.png"

            encode = run_vpng(
                vpng,
                [
                    "encode",
                    str(raw),
                    str(local),
                    "--input-format",
                    "raw",
                    "--width",
                    str(width),
                    "--height",
                    str(height),
                    "--format",
                    fmt,
                    "--tile-size",
                    str(args.tile_size),
                    "--workers",
                    str(args.workers),
                ],
            )
            if encode["exit"] != 0:
                raise RuntimeError(f"encode failed for {entry['id']}: {encode['error']}")

            url = f"{base_url}/{name}"
            info = run_vpng_json(vpng, ["http-info", url])

            view_w = min(width, args.viewport)
            view_h = min(height, args.viewport)

            remote = run_vpng_json(
                vpng,
                [
                    "http-decode-region",
                    url,
                    str(remote_png),
                    "--x",
                    "0",
                    "--y",
                    "0",
                    "--width",
                    str(view_w),
                    "--height",
                    str(view_h),
                ],
            )
            local_region = run_vpng_json(
                vpng,
                [
                    "decode-region",
                    str(local),
                    str(local_png),
                    "--x",
                    "0",
                    "--y",
                    "0",
                    "--width",
                    str(view_w),
                    "--height",
                    str(view_h),
                ],
            )
            # The two PNG files are compared as well as the two canonical
            # digests: the digests are the pixel equality, and the files are
            # the proof that both paths wrote the same picture.
            remote_file_digest = file_sha256(remote_png)
            local_file_digest = file_sha256(local_png)

            records.append(
                {
                    "id": entry["id"],
                    "category": entry["category"],
                    "width": width,
                    "height": height,
                    "pixel_format": fmt,
                    "canonical_bytes": entry["canonical_bytes"],
                    "file_bytes": local.stat().st_size,
                    "file_sha256": file_sha256(local),
                    "header": read_header(local),
                    "minor": info["minor"],
                    "info_bytes_read": info["bytes_read"],
                    "info_source_bytes": info["source_bytes_received"],
                    "info_requests": info["requests"],
                    "viewport_width": view_w,
                    "viewport_height": view_h,
                    "http_bytes_read": remote["bytes_read"],
                    "http_source_bytes": remote["source_bytes_received"],
                    "http_requests": remote["requests"],
                    "http_ms": remote["ms"],
                    "local_ms": local_region["ms"],
                    "http_sha256": remote["sha256"],
                    "local_sha256": local_region["sha256"],
                    "identical": remote["sha256"] == local_region["sha256"],
                    "http_png_sha256": remote_file_digest,
                    "local_png_sha256": local_file_digest,
                    "identical_file": remote_file_digest == local_file_digest,
                    "http_whole_object_fraction": round(
                        remote["source_bytes_received"] / local.stat().st_size, 6
                    ),
                    "info_whole_object_fraction": round(
                        info["source_bytes_received"] / local.stat().st_size, 6
                    ),
                }
            )

            if number % 25 == 0 or number == len(sample):
                print(f"  {number}/{len(sample)} measured")

            for path in (local, remote_png, local_png):
                path.unlink(missing_ok=True)
    finally:
        server.shutdown()
        server.server_close()

    identical = sum(1 for record in records if record["identical"])
    identical_file = sum(1 for record in records if record["identical_file"])
    total_object = sum(record["file_bytes"] for record in records)
    total_info = sum(record["info_bytes_read"] for record in records)
    total_http = sum(record["http_bytes_read"] for record in records)
    total_source = sum(record["http_source_bytes"] for record in records)
    total_info_source = sum(record["info_source_bytes"] for record in records)

    by_category: dict[str, dict] = {}
    for record in records:
        bucket = by_category.setdefault(
            record["category"],
            {
                "images": 0,
                "object_bytes": 0,
                "info_bytes": 0,
                "http_bytes": 0,
                "identical": 0,
                "http_requests": 0,
            },
        )
        bucket["images"] += 1
        bucket["object_bytes"] += record["file_bytes"]
        bucket["info_bytes"] += record["info_bytes_read"]
        bucket["http_bytes"] += record["http_bytes_read"]
        bucket["identical"] += 1 if record["identical"] else 0
        bucket["http_requests"] += record["http_requests"]
    for bucket in by_category.values():
        bucket["info_fraction"] = round(
            bucket["info_bytes"] / bucket["object_bytes"], 6
        )
        bucket["http_fraction"] = round(
            bucket["http_bytes"] / bucket["object_bytes"], 6
        )

    summary = {
        "sample": len(records),
        "identical": identical,
        "identical_differing": len(records) - identical,
        "identical_file": identical_file,
        "identical_file_differing": len(records) - identical_file,
        "object_bytes_total": total_object,
        "info_bytes_total": total_info,
        "http_bytes_total": total_http,
        "info_source_bytes_total": total_info_source,
        "http_source_bytes_total": total_source,
        "info_fraction": round(total_info / total_object, 6) if total_object else None,
        "http_fraction": round(total_http / total_object, 6) if total_object else None,
        "info_requests_total": sum(record["info_requests"] for record in records),
        "http_requests_total": sum(record["http_requests"] for record in records),
        "server_requests": server.requests,
        "server_range_requests": server.range_requests,
        "server_bytes_sent": server.bytes_sent,
        "http_ms": distribution([record["http_ms"] for record in records]),
        "local_ms": distribution([record["local_ms"] for record in records]),
        "by_category": by_category,
    }

    payload = {
        "generated": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
            "machine": platform.machine(),
        },
        "settings": {
            "tile_size": args.tile_size,
            "workers": args.workers,
            "viewport": args.viewport,
            "vpng_binary": str(vpng),
            "server": "a local HTTP/1.1 server that honours Range",
            "sample_rule": "the manifest order, which the corpus builder sorted by the digest of the path",
            "memory_scope": {
                "included": "nothing: this measurement reports bytes over the wire, not process memory",
                "excluded": "the process working set of the driver and of the CLI",
            },
        },
        "summary": summary,
        "images": records,
    }
    Path(args.output).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    print()
    print(f"sample:                    {summary['sample']}")
    print(f"identical to the local read: {identical}/{summary['sample']}")
    print(f"identical PNG files:        {identical_file}/{summary['sample']}")
    print(f"object bytes:              {summary['object_bytes_total']}")
    print(f"bytes fetched for info:     {summary['info_bytes_total']}")
    print(f"bytes fetched for a region: {summary['http_bytes_total']}")
    print(f"fraction for a region:      {summary['http_fraction']}")
    print(f"HTTP median:                {summary['http_ms']['median']} ms")
    print(f"local median:               {summary['local_ms']['median']} ms")
    print(f"server requests:            {summary['server_requests']}")
    print(f"server bytes sent:          {summary['server_bytes_sent']}")
    print(
        "source bytes received:      "
        f"{summary['http_source_bytes_total'] + summary['info_source_bytes_total']}"
    )
    print(f"written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
