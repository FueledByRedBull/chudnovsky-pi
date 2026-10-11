Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolchainBin = Join-Path $root ".tools/w64devkit/bin"
$m4Bin = Join-Path $root ".tools/msys-m4/usr/bin"
$gitUnixBin = Join-Path $root ".tools/git-usr-bin"

if (-not (Test-Path (Join-Path $toolchainBin "gcc.exe"))) {
    throw "w64devkit was not found. Expected gcc.exe under $toolchainBin"
}
if (-not (Test-Path (Join-Path $m4Bin "m4.exe"))) {
    throw "m4 was not found. Expected m4.exe under $m4Bin"
}

$gccLibDir = Get-ChildItem (Join-Path $root ".tools/w64devkit/lib/gcc/x86_64-w64-mingw32") -Directory |
    Sort-Object Name -Descending |
    Select-Object -First 1
if ($null -eq $gccLibDir) {
    throw "Could not find the w64devkit GCC runtime library directory"
}

$libgcc = Join-Path $gccLibDir.FullName "libgcc.a"
$libgccEh = Join-Path $gccLibDir.FullName "libgcc_eh.a"
if ((Test-Path $libgcc) -and -not (Test-Path $libgccEh)) {
    Copy-Item -LiteralPath $libgcc -Destination $libgccEh
}

$gitInstallUnixBin = "C:/Program Files/Git/usr/bin"
if (-not (Test-Path (Join-Path $gitInstallUnixBin "msys-2.0.dll"))) {
    throw "Git for Windows MSYS runtime was not found at $gitInstallUnixBin"
}
if (-not (Test-Path $gitUnixBin)) {
    New-Item -ItemType Junction -Path $gitUnixBin -Target $gitInstallUnixBin | Out-Null
}

$env:PATH = "$gitUnixBin;$toolchainBin;$m4Bin;$env:PATH"
# GMP CPU name: picks its hand-tuned assembly and FFT tables. zen4 matches the
# Ryzen 7 7800X3D; use "x86_64" for a generic build that runs on any CPU.
$gmpCpu = "zen4"
$env:build_alias = "$gmpCpu-w64-mingw32"
$env:ac_cv_build = "$gmpCpu-w64-mingw32"
$env:GMP_MPFR_SYS_CACHE = "_"
# Linking needs the FLINT libraries from build_flint.sh. On a fresh checkout the
# first run builds GMP and then fails to link; run build_flint.sh and build again.
# Extra arguments go to cargo.
cargo +stable-x86_64-pc-windows-msvc build --release --target x86_64-pc-windows-gnu @args
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
}
