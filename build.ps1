param([string]$Cpu = "zen4")
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$tools = Join-Path $root ".tools"
$toolchainBin = Join-Path $tools "w64devkit/bin"
$m4Dir = Join-Path $tools "msys-m4"
$m4Bin = Join-Path $m4Dir "usr/bin"
$flintSrc = Join-Path $tools "flint-src"
$crateDir = Join-Path $tools "gmp-mpfr-sys"
$gitUnixBin = Join-Path $tools "git-usr-bin"
$tar = "$env:SystemRoot\System32\tar.exe"

$gitInstallUnixBin = "C:/Program Files/Git/usr/bin"
if (-not (Test-Path (Join-Path $gitInstallUnixBin "msys-2.0.dll"))) {
    throw "Git for Windows MSYS runtime was not found at $gitInstallUnixBin"
}

# Downloads an archive into .tools unless it is already there, then checks its
# SHA-256 before anything uses it. Returns the archive path.
function Get-Archive($Name, $Url, $Sha256) {
    $file = Join-Path $tools $Name
    if (-not (Test-Path $file)) {
        Write-Host "Downloading $Name"
        curl.exe -fL -o $file $Url
        if ($LASTEXITCODE -ne 0) {
            throw "Downloading $Url failed with exit code $LASTEXITCODE"
        }
    }
    if ((Get-FileHash $file -Algorithm SHA256).Hash -ne $Sha256) {
        throw "SHA-256 of $file does not match the expected $Sha256; delete the file and run again"
    }
    return $file
}

New-Item -ItemType Directory -Force $tools | Out-Null

if (-not (Test-Path (Join-Path $toolchainBin "gcc.exe"))) {
    $sfx = Get-Archive "w64devkit-x64-2.8.0.7z.exe" `
        "https://github.com/skeeto/w64devkit/releases/download/v2.8.0/w64devkit-x64-2.8.0.7z.exe" `
        "6252bf34fe2231a55ac7f03d482b36d2c7c58697990551bba508102cfb3f342e"
    Write-Host "Unpacking w64devkit"
    # A 7-Zip self-extractor: -o is the output folder, -y answers yes to prompts.
    $unpack = Start-Process $sfx -ArgumentList "-o`"$tools`"", "-y" -Wait -PassThru
    if ($unpack.ExitCode -ne 0) {
        throw "Unpacking w64devkit failed with exit code $($unpack.ExitCode)"
    }
}

if (-not (Test-Path (Join-Path $m4Bin "m4.exe"))) {
    $m4Archive = Get-Archive "m4-1.4.21-1-x86_64.pkg.tar.zst" `
        "https://repo.msys2.org/msys/x86_64/m4-1.4.21-1-x86_64.pkg.tar.zst" `
        "b561e8871c7599502d9fae7196ee965b3fed3d7af09154eb8d2f5e4d9e5e5106"
    New-Item -ItemType Directory -Force $m4Dir | Out-Null
    # Windows' tar reads zstd; the tar that Git puts on PATH does not.
    & $tar -xf $m4Archive -C $m4Dir usr/bin/m4.exe
    if ($LASTEXITCODE -ne 0) {
        throw "Unpacking m4 failed with exit code $LASTEXITCODE"
    }
}

if (-not (Test-Path (Join-Path $flintSrc "flint-3.6.0"))) {
    $flintArchive = Get-Archive "flint-3.6.0.tar.gz" `
        "https://github.com/flintlib/flint/releases/download/v3.6.0/flint-3.6.0.tar.gz" `
        "b95e2c7792f5eea4a1c8d2d42c4098434756832e57a094b295eb5dfdc9b4c36b"
    New-Item -ItemType Directory -Force $flintSrc | Out-Null
    & $tar -xf $flintArchive -C $flintSrc
    if ($LASTEXITCODE -ne 0) {
        throw "Unpacking FLINT failed with exit code $LASTEXITCODE"
    }
}

if (-not (Test-Path $crateDir)) {
    $crateArchive = Get-Archive "gmp-mpfr-sys-1.7.1.crate" `
        "https://static.crates.io/crates/gmp-mpfr-sys/gmp-mpfr-sys-1.7.1.crate" `
        "7db155b537cb791b133341f99f68371d86ee7fa4c79aacfbc376d72d23c70531"
    & $tar -xf $crateArchive -C $tools
    if ($LASTEXITCODE -ne 0) {
        throw "Unpacking gmp-mpfr-sys failed with exit code $LASTEXITCODE"
    }
    $unpacked = Join-Path $tools "gmp-mpfr-sys-1.7.1"
    & "$gitInstallUnixBin/patch.exe" -p1 -d $unpacked -i (Join-Path $root "patches/gmp-mpfr-sys-1.7.1.patch")
    if ($LASTEXITCODE -ne 0) {
        throw "Patching gmp-mpfr-sys failed with exit code $LASTEXITCODE"
    }
    Rename-Item $unpacked "gmp-mpfr-sys"
}

$gccLibDir = Get-ChildItem (Join-Path $tools "w64devkit/lib/gcc/x86_64-w64-mingw32") -Directory |
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

if (-not (Test-Path $gitUnixBin)) {
    New-Item -ItemType Junction -Path $gitUnixBin -Target $gitInstallUnixBin | Out-Null
}

$env:PATH = "$gitUnixBin;$toolchainBin;$m4Bin;$env:PATH"
# GMP CPU name: picks its hand-tuned assembly and FFT tables. zen4 matches the
# Ryzen 7 7800X3D. "generic" leaves build_alias unset, which gives a portable
# GMP that chooses its code at run time.
if ($Cpu -eq "generic") {
    Remove-Item Env:build_alias -ErrorAction SilentlyContinue
    Remove-Item Env:ac_cv_build -ErrorAction SilentlyContinue
} else {
    $env:build_alias = "$Cpu-w64-mingw32"
    $env:ac_cv_build = "$Cpu-w64-mingw32"
}
$env:GMP_MPFR_SYS_CACHE = "_"
# Panic locations are compiled into the binary; keep local paths out of them.
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
# The encoded form separates flags with 0x1f, so paths may contain spaces.
$env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$cargoHome=cargo" + [char]0x1f + "--remap-path-prefix=$root=pi_calc"

# The FLINT and MPFR libraries are built once, for one CPU choice, and linking
# needs them. The stamp records which choice.
$stamp = Join-Path $tools "flint-deps/cpu.txt"
if (Test-Path $stamp) {
    $built = (@(Get-Content $stamp) -join "").Trim()
    if ($built -cne $Cpu) {
        throw "The libraries in .tools were built for '$built', not '$Cpu'. Switching needs a fresh checkout, or delete .tools/flint-deps, .tools/flint-src and target."
    }
} else {
    Write-Host "Building GMP, MPFR and FLINT"
    sh build_flint.sh $Cpu
    if ($LASTEXITCODE -ne 0) {
        throw "build_flint.sh failed with exit code $LASTEXITCODE"
    }
    Set-Content -Path $stamp -Value $Cpu -Encoding ASCII
}

# Extra arguments go to cargo.
cargo +stable-x86_64-pc-windows-msvc build --release --target x86_64-pc-windows-gnu @args
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
}
