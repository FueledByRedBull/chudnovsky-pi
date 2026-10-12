#!/bin/sh
# Builds static GMP, MPFR and FLINT 3, the arithmetic backend of pi_calc.
# Run by build.ps1, which also fetches the sources into .tools:
#   sh build_flint.sh CPU      (a GMP CPU name such as zen4, or generic)
set -eu

root=$(cd "$(dirname "$0")" && pwd)
# Same order as build.ps1. Git's MSYS tools come first because configure scripts
# break on the busybox sed and sh in w64devkit, and they come through the
# git-usr-bin junction because make cannot run a shell whose path has a space.
export PATH="$root/.tools/git-usr-bin:$root/.tools/w64devkit/bin:$root/.tools/msys-m4/usr/bin:$PATH"

# Same CPU choice as build.ps1: FLINT maps it to -march and its tuned parameters.
cpu=${1:?usage: sh build_flint.sh CPU}
if [ "$cpu" = generic ]; then triple=x86_64-w64-mingw32; else triple=$cpu-w64-mingw32; fi
build=$root/.tools/flint-src
deps=$root/.tools/flint-deps
jobs=$(nproc)

# FLINT wants GMP and MPFR under one prefix.
mkdir -p "$deps/include" "$deps/lib"

# Source paths stay relative: the mingw make cannot follow MSYS absolute paths.
gmp_src=../../gmp-mpfr-sys/gmp-6.3.0-c
mkdir -p "$build/gmp-build"
cd "$build/gmp-build"
# A generic GMP carries code for every CPU and picks at run time.
if [ "$cpu" = generic ]; then fat=--enable-fat; else fat=; fi
"$gmp_src/configure" --build=$triple --host=$triple --disable-shared --with-pic $fat
make -j"$jobs"
# The make in w64devkit cuts a command line at 8191 characters, and the one
# that archives libgmp.a is longer: the last objects (scanf/ and rand/ at
# least) are left out without an error. The archive is put together here
# from the object list of that command instead.
make -n -B libgmp.la | grep -e '--mode=link' | tr ' ' '\n' | sed -n 's/\.lo$/.o/p' > objects.txt
rm -f "$deps/lib/libgmp.a"
ar crs "$deps/lib/libgmp.a" @objects.txt
# gmp-mpfr-sys asks pkg-config where GMP is.
mkdir -p "$deps/lib/pkgconfig"
cat > "$deps/lib/pkgconfig/gmp.pc" <<EOF
Name: gmp
Description: GNU MP
Version: 6.3.0
Libs: -L$(cd "$deps/lib" && pwd -W) -lgmp
EOF
cp gmp.h "$deps/include/"

mpfr_src=../../gmp-mpfr-sys/mpfr-4.2.2-c
mkdir -p "$build/mpfr-build"
cd "$build/mpfr-build"
"$mpfr_src/configure" --build=x86_64-w64-mingw32 --host=x86_64-w64-mingw32 \
    --with-gmp="$deps" --disable-shared --enable-static
make -j"$jobs"
cp src/.libs/libmpfr.a "$deps/lib/"
cp "$mpfr_src/src/mpfr.h" "$deps/include/"

# Leaves libflint.a in the source directory, where build.rs looks for it.
cd "$build/flint-3.6.0"
# FLINT compiles its sources by absolute path and keeps their names for error
# messages; the mapping keeps the local path out of the binary.
win_root=$(cd "$root" && pwd -W)
./configure CC="gcc -ffile-prefix-map=$win_root=pi_calc" \
    --build=$triple --host=$triple --with-gmp="$deps" --with-mpfr="$deps" \
    --disable-shared --enable-static --enable-avx2 --disable-debug
# FLINT's Makefile records absolute MSYS paths, which the mingw make cannot follow.
sed -i "s|$root|$win_root|g" Makefile
make -j"$jobs"
