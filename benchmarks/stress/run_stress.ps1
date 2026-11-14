# Large-image stress harness for VPNG.
#
# Records wall time, sampled peak working set, file sizes, canonical SHA-256 and
# exactness for native large images. Raw pixel buffers are written to the
# git-ignored results directory and deleted after each case.
#
# Usage (from the repository root):
#   powershell -NoProfile -ExecutionPolicy Bypass -File benchmarks/stress/run_stress.ps1
#   ... -SkipPhoto              # synthetic cases only
#   ... -Attempt16KRgba8        # opt in to the 1 GiB RGBA8 case

param(
    [string]$Vpng = "$PSScriptRoot\..\..\target\release\vpng.exe",
    [switch]$SkipPhoto,
    [switch]$SkipSynthetic,
    [switch]$Attempt16KRgba8,
    [int]$TimeoutSec = 900
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not (Test-Path $Vpng)) {
    throw "vpng binary not found: $Vpng (build with: cargo build --release --workspace)"
}
$resultsDir = Join-Path $repo "benchmarks\results"
$work = Join-Path $resultsDir "stress-work"
$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
$resultsPath = Join-Path $resultsDir "stress_$stamp.json"

New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force -Path $work | Out-Null

$steps = [System.Collections.Generic.List[object]]::new()
$cases = [System.Collections.Generic.List[object]]::new()

function Get-FreeMb {
    [int]((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024)
}

function Invoke-Step {
    param([string]$Label, [string[]]$VpngArgs)

    # System.Diagnostics.Process is used instead of Start-Process because the
    # latter does not populate ExitCode when output is redirected.
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $Vpng
    $quoted = foreach ($arg in $VpngArgs) {
        if ($arg -match '[\s"]') { '"' + ($arg -replace '"', '\"') + '"' } else { $arg }
    }
    $psi.Arguments = ($quoted -join " ")
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $proc = [System.Diagnostics.Process]::new()
    $proc.StartInfo = $psi
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    [void]$proc.Start()
    $stdoutTask = $proc.StandardOutput.ReadToEndAsync()
    $stderrTask = $proc.StandardError.ReadToEndAsync()

    $peak = 0L
    while (-not $proc.HasExited) {
        try {
            $proc.Refresh()
            if ($proc.PeakWorkingSet64 -gt $peak) { $peak = $proc.PeakWorkingSet64 }
        } catch { }
        if ($sw.Elapsed.TotalSeconds -gt $TimeoutSec) {
            try { $proc.Kill() } catch { }
            break
        }
        Start-Sleep -Milliseconds 20
    }
    $proc.WaitForExit()
    $sw.Stop()

    $joined = (($stdoutTask.Result + $stderrTask.Result)).Trim()
    $step = [pscustomobject]@{
        case         = $Label.Split(":")[0]
        step         = $Label.Split(":")[1]
        exit         = $proc.ExitCode
        ms           = [int]$sw.Elapsed.TotalMilliseconds
        peak_rss_mb  = [math]::Round($peak / 1MB, 1)
        output       = if ($joined.Length -gt 400) { $joined.Substring(0, 400) } else { $joined }
        args         = ($VpngArgs -join " ")
    }
    $steps.Add($step)
    $status = if ($step.exit -eq 0) { "ok" } else { "exit $($step.exit)" }
    Write-Host ("  {0,-28} {1,-9} {2,8} ms  peak {3,7} MB" -f $step.step, $status, $step.ms, $step.peak_rss_mb)
    return $step
}

function Get-CodecHistogram {
    param([string]$Path)
    $histogram = @{}
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $reader = [System.IO.BinaryReader]::new($stream)
        $stream.Position = 28
        $tiles = $reader.ReadUInt32()
        $stream.Position = 32
        $directoryOffset = [int]$reader.ReadUInt64()
        for ($i = 0; $i -lt $tiles; $i++) {
            $stream.Position = $directoryOffset + ($i * 40) + 12
            $codec = $reader.ReadByte()
            $key = "$codec"
            if ($histogram.ContainsKey($key)) { $histogram[$key]++ } else { $histogram[$key] = 1 }
        }
    } finally {
        $stream.Dispose()
    }
    return $histogram
}

function Invoke-ImageCase {
    param(
        [string]$Name,
        [string]$RawPath,
        [int]$Width,
        [int]$Height,
        [string]$Format,
        [object]$Source
    )

    $rawBytes = (Get-Item $RawPath).Length
    $base = @(
        "--input-format", "raw",
        "--width", "$Width",
        "--height", "$Height",
        "--format", $Format,
        "--tile-size", "256"
    )
    $rawVpng = Join-Path $work "$Name-raw.vpng"
    $autoVpng = Join-Path $work "$Name-auto.vpng"
    $regionPng = Join-Path $work "$Name-region.png"
    $freeBefore = Get-FreeMb

    Write-Host ""
    Write-Host "== $Name  ${Width}x${Height} $Format  source $($Source.bytes) bytes, free ${freeBefore} MB"

    $encodeRaw = Invoke-Step "$Name`:encode-raw" (@("encode", $RawPath, $rawVpng) + $base + @("--codec", "raw"))
    if ($encodeRaw.exit -ne 0) {
        $cases.Add([pscustomobject]@{ name = $Name; source = $Source; status = "encode-raw failed"; raw_bytes = $rawBytes })
        return
    }
    $check = Invoke-Step "$Name`:check-raw" @("check", $rawVpng)
    $verify = Invoke-Step "$Name`:verify-raw" (@("verify", $RawPath, $rawVpng) + $base)
    $region = Invoke-Step "$Name`:region-raw" @("decode-region", $rawVpng, $regionPng, "--x", "64", "--y", "64", "--width", "512", "--height", "512")

    $encodeAuto = Invoke-Step "$Name`:encode-auto" (@("encode", $RawPath, $autoVpng) + $base)
    $verifyAuto = $null
    $histogram = @{}
    if ($encodeAuto.exit -eq 0) {
        $verifyAuto = Invoke-Step "$Name`:verify-auto" (@("verify", $RawPath, $autoVpng) + $base)
        $checkAuto = Invoke-Step "$Name`:check-auto" @("check", $autoVpng)
        $histogram = Get-CodecHistogram $autoVpng
    }

    $rawSize = (Get-Item $rawVpng).Length
    $autoSize = if (Test-Path $autoVpng) { (Get-Item $autoVpng).Length } else { $null }
    $freeAfter = Get-FreeMb

    $case = [pscustomobject]@{
        name             = $Name
        status           = "ok"
        source           = $Source
        width            = $Width
        height           = $Height
        pixel_format     = $Format
        canonical_bytes  = $rawBytes
        raw_vpng_bytes   = $rawSize
        auto_vpng_bytes  = $autoSize
        auto_vs_raw      = if ($autoSize) { [math]::Round($autoSize / $rawSize, 4) } else { $null }
        auto_codecs      = $histogram
        check_exit       = $check.exit
        verify_exit      = $verify.exit
        verify_auto_exit = if ($verifyAuto) { $verifyAuto.exit } else { $null }
        region_exit      = $region.exit
        free_mb_before   = $freeBefore
        free_mb_after    = $freeAfter
    }
    $cases.Add($case)

    foreach ($file in @($RawPath, $rawVpng, $autoVpng, $regionPng)) {
        Remove-Item $file -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "vpng stress harness"
Write-Host "  vpng:    $Vpng"
Write-Host "  results: $resultsPath"
Write-Host ("  host:    {0} logical CPUs, {1} MB physical, {2} MB free" -f `
    [Environment]::ProcessorCount, `
    [int]((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1MB), (Get-FreeMb))

if (-not $SkipPhoto) {
    $photo = Join-Path $repo "1006677.jpg"
    if (Test-Path $photo) {
        $raw = Join-Path $work "photo.rgb"
        $meta = python (Join-Path $PSScriptRoot "canonicalize.py") $photo $raw | ConvertFrom-Json
        Write-Host ("  canonicalized photo: {0} bytes, sha256 {1}" -f $meta.bytes, $meta.sha256)
        Invoke-ImageCase -Name "photo" -RawPath $raw -Width $meta.width -Height $meta.height `
            -Format $meta.pixel_format -Source $meta
    } else {
        Write-Host "  photo fixture not found; skipping"
    }
}

if (-not $SkipSynthetic) {
    $gen = Join-Path $PSScriptRoot "gen_synthetic.py"

    $g8 = Join-Path $work "gray8-16k.raw"
    $meta = python $gen gray8-gradient 16384 16384 $g8 | ConvertFrom-Json
    Write-Host ("  generated 16K gray8: {0} bytes, sha256 {1}" -f $meta.bytes, $meta.sha256)
    Invoke-ImageCase -Name "gray8-16k" -RawPath $g8 -Width 16384 -Height 16384 `
        -Format "gray8" -Source $meta

    $rgba = Join-Path $work "rgba8-8k.raw"
    $meta = python $gen rgba8-blocks 8192 8192 $rgba | ConvertFrom-Json
    Write-Host ("  generated 8K rgba8: {0} bytes, sha256 {1}" -f $meta.bytes, $meta.sha256)
    Invoke-ImageCase -Name "rgba8-8k" -RawPath $rgba -Width 8192 -Height 8192 `
        -Format "rgba8" -Source $meta
}

if ($Attempt16KRgba8) {
    $free = Get-FreeMb
    if ($free -lt 5120) {
        Write-Host "  skipping 16K RGBA8: ${free} MB free, need >= 5120 MB for 1 GiB input + output"
        $cases.Add([pscustomobject]@{ name = "rgba8-16k"; status = "skipped: insufficient free RAM"; free_mb = $free })
    } else {
        $gen = Join-Path $PSScriptRoot "gen_synthetic.py"
        $rgba = Join-Path $work "rgba8-16k.raw"
        $meta = python $gen rgba8-blocks 16384 16384 $rgba | ConvertFrom-Json
        Invoke-ImageCase -Name "rgba8-16k" -RawPath $rgba -Width 16384 -Height 16384 `
            -Format "rgba8" -Source $meta
    }
}

$report = [pscustomobject]@{
    generated_at = (Get-Date).ToString("o")
    vpng         = $Vpng
    host_cpus    = [Environment]::ProcessorCount
    host_ram_mb  = [int]((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1MB)
    cases        = $cases
    steps        = $steps
}
$report | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 $resultsPath

Write-Host ""
Write-Host "== summary =="
$cases | Format-Table name, status, canonical_bytes, raw_vpng_bytes, auto_vpng_bytes, auto_vs_raw -AutoSize | Out-String -Width 240
Write-Host "report: $resultsPath"
Write-Host "note: peak RSS is sampled every ~20 ms, so it is a lower bound on the true peak."