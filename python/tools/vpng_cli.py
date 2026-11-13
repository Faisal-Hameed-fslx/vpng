"""Run the Rust `vpng` CLI and parse its JSON output.

This is a thin process wrapper. It deliberately contains no format logic so that
Python can never diverge from the Rust implementation.
"""

from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

EXIT_CODES = {
    0: "success",
    1: "generic failure",
    2: "invalid arguments",
    3: "unsupported feature",
    4: "corrupt file",
    5: "resource limit exceeded",
    6: "size limit exceeded",
    7: "verification failure",
    8: "io failure",
}


@dataclass
class CliResult:
    argv: Sequence[str]
    returncode: int
    stdout: str
    stderr: str
    json: dict | None

    @property
    def ok(self) -> bool:
        return self.returncode == 0

    @property
    def status(self) -> str:
        return EXIT_CODES.get(self.returncode, f"unknown({self.returncode})")


def cli_path() -> str:
    """Locate the built binary, preferring a release build."""
    found = shutil.which("vpng")
    if found:
        return found
    for profile in ("release", "debug"):
        candidate = Path("target") / profile / "vpng.exe"
        if candidate.exists():
            return str(candidate)
    raise FileNotFoundError(
        "vpng binary not found; build it with `cargo build -p vpng-cli`"
    )


def run(*args: str, json_output: bool = False) -> CliResult:
    argv = [cli_path(), *args]
    if json_output and "--json" not in argv:
        argv.append("--json")
    proc = subprocess.run(argv, capture_output=True, text=True)
    parsed = None
    if json_output and proc.stdout.strip():
        try:
            parsed = json.loads(proc.stdout)
        except json.JSONDecodeError:
            parsed = None
    return CliResult(argv, proc.returncode, proc.stdout, proc.stderr, parsed)


def encode(input_path: str, output_path: str, **options: object) -> CliResult:
    args = ["encode", input_path, output_path]
    want_json = bool(options.pop("json", True))
    for key, value in options.items():
        flag = f"--{key.replace('_', '-')}"
        if isinstance(value, bool):
            # Boolean options are switches, not `--flag true` pairs.
            if value:
                args.append(flag)
        else:
            args.extend([flag, str(value)])
    return run(*args, json_output=want_json)


def verify(original: str, candidate: str) -> CliResult:
    return run("verify", original, candidate)
