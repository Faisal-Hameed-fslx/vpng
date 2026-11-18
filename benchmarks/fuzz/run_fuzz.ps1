# Fuzz every target with cargo-fuzz, one target at a time.
#
# The settings match the CI job in `.github/workflows/ci.yml`: the same
# seven targets, the same `-max_total_time`, and the same RSS limit. The
# toolchain is pinned to a dated nightly, because the newest nightly on this
# host has a Windows regression that crashes build scripts.
#
# `--fuzz-dir` is cargo-fuzz's *fuzz project* directory (the one holding the
# fuzz crate's Cargo.toml), not the corpus directory; the corpus lives in
# `<fuzz-dir>/corpus/<target>` and the crash artifacts in
# `<fuzz-dir>/artifacts/<target>`.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File benchmarks/fuzz/run_fuzz.ps1
#   powershell -ExecutionPolicy Bypass -File benchmarks/fuzz/run_fuzz.ps1 -Seconds 60

param(
    [int]$Seconds = 120,
    [int]$RssLimitMb = 4096,
    [string]$Toolchain = "nightly-2026-10-01",
    [string[]]$Targets = @(
        "header", "directory", "container", "tile_payload",
        "experimental_codec", "region", "editing"
    )
)

$ErrorActionPreference = "Continue"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $repo
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One fuzzing worker at a time: this host has 8 GB of RAM and a small
# page file, and an AddressSanitizer build needs the address space.
$env:CARGO_BUILD_JOBS = "1"

$fuzzDir = Join-Path $repo "fuzz"
$report = Join-Path $repo "benchmarks\results\fuzz-cargo-fuzz-report.json"
New-Item -ItemType Directory -Force -Path (Split-Path $report) | Out-Null

$results = @()
foreach ($target in $Targets) {
    Write-Host "== $target"
    $started = Get-Date
    $output = & cargo "+$Toolchain" fuzz run $target --fuzz-dir $fuzzDir -- `
        "-max_total_time=$Seconds" "-rss_limit_mb=$RssLimitMb" 2>&1
    $code = $LASTEXITCODE
    $elapsed = [int]((Get-Date) - $started).TotalSeconds
    $text = ($output | Out-String)

    # libFuzzer prints "Done <n> runs in <t> second(s)" at the end of a clean
    # run and "stat::number_of_executed_units: <n>" in its statistics block.
    $execs = 0
    if ($text -match "stat::number_of_executed_units:\s+(\d+)") {
        $execs = [int]$Matches[1]
    }
    elseif ($text -match "Done\s+(\d+)\s+runs") {
        $execs = [int]$Matches[1]
    }
    elseif ($text -match "(\d+)\s+executions") {
        $execs = [int]$Matches[1]
    }

    $artifactDir = Join-Path $fuzzDir "artifacts\$target"
    $crashes = (Get-ChildItem $artifactDir -Filter "crash-*" -ErrorAction SilentlyContinue).Count
    $status = if ($code -eq 0 -and $crashes -eq 0) { "PASS" } else { "FAIL" }
    Write-Host "   $status  exit=$code  ${elapsed}s  execs=$execs  crashes=$crashes"
    if ($text) { Write-Host ($text.TrimEnd() -split "`n" | Select-Object -Last 12 | Out-String) }

    $results += [ordered]@{
        target        = $target
        status        = $status
        exit_code     = $code
        elapsed_s     = $elapsed
        executions    = $execs
        crash_files   = $crashes
        output_tail   = ($text.TrimEnd() -split "`n" | Select-Object -Last 20) -join "`n"
    }
}

$document = [ordered]@{
    tool        = "cargo-fuzz"
    toolchain   = $Toolchain
    generated_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    max_total_time_s = $Seconds
    rss_limit_mb      = $RssLimitMb
    targets      = $results
}
$document | ConvertTo-Json -Depth 6 | Set-Content -Path $report -Encoding utf8
Write-Host "wrote $report"

$failed = ($results | Where-Object { $_.status -ne "PASS" }).Count
Write-Host ""
Write-Host "PASS $($results.Count - $failed)  FAIL $failed"
exit $failed
