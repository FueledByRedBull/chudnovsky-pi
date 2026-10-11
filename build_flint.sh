#!/bin/sh
# Builds static MPFR and FLINT 3, the arithmetic backend of pi_calc.
# Run from Git Bash after a first build.ps1, which produces the libgmp.a used
# here (and cannot link until this script has run):
#   sh build_flint.sh
# Expects the FLINT release unpacked at .tools/flint-src/flint-3.6.0
# (https://github.com/flintlib/flint/releases/download/v3.6.0/flint-3.6.0.tar.gz).
set -eu

root=$(cd "$(dirname "$0")" && pwd)
# Same order as build.ps1. Git's MSYS tools come first because configure scripts
# break on the busybox sed and sh in w64devkit, and they come through the
# git-usr-bin junction because make cannot run a shell whose path has a space.
export PATH="$root/.tools/git-usr-bin:$root/.tools/w64devkit/bin:$root/.tools/msys-m4/usr/bin:$PATH"

# Same CPU choice as build.ps1: FLINT maps it to -march and its tuned parameters.
triple=zen4-w64-mingw32
build=$root/.tools/flint-src
deps=$root/.tools/flint-deps
jobs=$(nproc)

# FLINT wants GMP and MPFR under one prefix; GMP comes from the cargo build.
gmp_out=$(ls -td "$root"/target/x86_64-pc-windows-gnu/release/build/gmp-mpfr-sys-*/out | head -1)
mkdir -p "$deps/include" "$deps/lib"
cp "$gmp_out/include/gmp.h" "$deps/include/"
cp "$gmp_out/lib/libgmp.a" "$deps/lib/"

# Source paths stay relative: the mingw make cannot follow MSYS absolute paths.
#
# On this toolchain libtool leaves the last objects (scanf/ and rand/) out of
# libgmp.a, and FLINT references GMP's random-number functions. A second GMP
# build supplies those objects as their own archive.
gmp_src=../../../vendor/gmp-mpfr-sys/gmp-6.3.0-c
mkdir -p "$build/gmp-build"
cd "$build/gmp-build"
"$gmp_src/configure" --build=$triple --host=$triple --disable-shared --with-pic
make -j"$jobs"
ar crs "$deps/lib/libgmprand.a" rand/*.o

mpfr_src=../../../vendor/gmp-mpfr-sys/mpfr-4.2.2-c
mkdir -p "$build/mpfr-build"
cd "$build/mpfr-build"
"$mpfr_src/configure" --build=x86_64-w64-mingw32 --host=x86_64-w64-mingw32 \
    --with-gmp="$deps" --disable-shared --enable-static
make -j"$jobs"
cp src/.libs/libmpfr.a "$deps/lib/"
cp "$mpfr_src/src/mpfr.h" "$deps/include/"

# Leaves libflint.a in the source directory, where build.rs looks for it.
cd "$build/flint-3.6.0"
./configure --build=$triple --host=$triple --with-gmp="$deps" --with-mpfr="$deps" \
    --disable-shared --enable-static --enable-avx2 --disable-debug
# FLINT's Makefile records absolute MSYS paths, which the mingw make cannot follow.
sed -i "s|$root|$(cd "$root" && pwd -W)|g" Makefile
make -j"$jobs"
