# Build a libFuzzer runtime for this host's target triple.
#
# Why this exists
# ---------------
# `libfuzzer-sys` compiles LLVM's libFuzzer C++ sources from the crate
# itself. On Windows it compiles `FuzzerExtFunctionsWindows.cpp`, which
# declares the optional sanitizer entry points with
#
#     __pragma(comment(linker, "/alternatename:..."))
#
# That is an MSVC-only construct: GCC and Clang both reject it for the
# `x86_64-w64-windows-gnu` target, so libFuzzer cannot be built here with
# either compiler. Upstream LLVM has the same limitation - libFuzzer on
# Windows is an MSVC-only configuration.
#
# This script therefore builds the runtime itself from the very same
# sources `libfuzzer-sys` would have compiled, with exactly one change:
# the MSVC `/alternatename` pragma is replaced by the equivalent weak
# alias that upstream uses on the other platforms. The result is a
# `libfuzzer.a` that `libfuzzer-sys` links through `CUSTOM_LIBFUZZER_PATH`
# instead of compiling its own copy.
#
# Nothing in the repository is modified: the sources are copied into
# benchmarks/results/libfuzzer-build/ and patched there.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File benchmarks/fuzz/build_libfuzzer.ps1

param(
    [string]$Clang = "C:\dev\toolchains\mingw64\bin\clang++.exe",
    [string]$Ar = "C:\dev\toolchains\mingw64\bin\ar.exe"
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $repo

if (-not (Test-Path $Clang)) { throw "clang++ not found at $Clang" }
if (-not (Test-Path $Ar)) { throw "ar not found at $Ar" }

# ---- locate the libfuzzer-sys sources ----------------------------------------
$registry = Join-Path $env:USERPROFILE ".cargo\registry\src"
$candidates = Get-ChildItem $registry -Directory -ErrorAction SilentlyContinue |
    ForEach-Object { Get-ChildItem $_.FullName -Directory -Filter "libfuzzer-sys-*" -ErrorAction SilentlyContinue } |
    Sort-Object Name -Descending
if (-not $candidates) { throw "libfuzzer-sys sources are not in the cargo registry" }

$work = Join-Path $repo "benchmarks\results\libfuzzer-build"
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force -Path $work | Out-Null

$source = $null
foreach ($candidate in $candidates) {
    $dir = Join-Path $candidate.FullName "libfuzzer"
    if (Test-Path (Join-Path $dir "FuzzerExtFunctionsWindows.cpp")) {
        $source = $dir
        Write-Host "using libfuzzer-sys sources from $($candidate.FullName)"
        break
    }
}
if (-not $source) { throw "no libfuzzer sources found" }

Copy-Item "$source\*" $work -Recurse -Force

# ---- patch the one MSVC-only construct ---------------------------------------
$ext = Join-Path $work "FuzzerExtFunctionsWindows.cpp"
$text = Get-Content $ext -Raw
$pragma = @'
#define EXTERNAL_FUNC(Name, Default)                                           \
  __pragma(comment(linker, "/alternatename:" WIN_SYM_PREFIX STRINGIFY(         \
                               Name) "=" WIN_SYM_PREFIX STRINGIFY(Default)))
'@
$alias = @'
// Patched for non-MSVC toolchains (see benchmarks/fuzz/build_libfuzzer.ps1):
// the weak alias is the portable equivalent of the MSVC /alternatename
// pragma, and it resolves to the same "function not defined" stub.
#define EXTERNAL_FUNC(Name, Default) __attribute__((weak, alias(#Default)))
'@
if (-not $text.Contains($pragma)) {
    throw "the expected EXTERNAL_FUNC pragma is not in $ext; the upstream layout changed"
}
$text = $text.Replace($pragma, $alias)
Set-Content -Path $ext -Value $text -NoNewline
Write-Host "patched $ext"

# ---- compile ----------------------------------------------------------------
$objects = @()
Get-ChildItem $work -Filter "*.cpp" | ForEach-Object {
    $obj = Join-Path $work ($_.BaseName + ".o")
    & $Clang -O2 -std=c++17 -w -I $work -c $_.FullName -o $obj
    if ($LASTEXITCODE -ne 0) { throw "failed to compile $($_.Name)" }
    $objects += $obj
}
Write-Host "compiled $($objects.Count) translation units"

# ---- archive -----------------------------------------------------------------
$archive = Join-Path $work "libfuzzer.a"
if (Test-Path $archive) { Remove-Item $archive -Force }
& $Ar rcs $archive @objects
if ($LASTEXITCODE -ne 0) { throw "failed to archive $archive" }

$size = [math]::Round((Get-Item $archive).Length / 1MB, 2)
Write-Host "wrote $archive ($size MB)"

# The runner reads this file so that both scripts agree on the archive.
$pointer = Join-Path $repo "benchmarks\results\libfuzzer-path.txt"
Set-Content -Path $pointer -Value $archive -Encoding ascii
Write-Host "wrote $pointer"
exit 0
