# chudnovsky-pi

A multithreaded pi calculator in Rust. It computes 100 million decimal digits
in about five seconds on an 8-core desktop CPU, using the Chudnovsky series
with binary splitting on top of [FLINT 3](https://flintlib.org/) and
[GMP](https://gmplib.org/).

```text
> pi_calc --digits 100M
Binary splitting: 3.047s
Final division: 0.643s
Decimal conversion: 0.793s
File write: 0.237s
Total: 4.723s
```

## Performance

Measured on a Ryzen 7 7800X3D (8 cores, 16 threads) under Windows 11, ten
runs per size, timed by the program from start to the end of the file write.

| Digits | Median | Fastest | Slowest |
|---|---|---|---|
| 10 thousand | 0.008 s | 0.007 s | 0.010 s |
| 1 million | 0.090 s | 0.087 s | 0.093 s |
| 10 million | 0.72 s | 0.68 s | 0.76 s |
| 100 million | 4.99 s | 4.67 s | 5.29 s |

These are not idle-machine figures: other programs were using 65 to 70% of
the CPU during the runs. An earlier set of ten 100-million-digit runs, at
about 59% load, ranged from 4.62 s to 4.92 s.

The first version of this program took 290 s for 100 million digits.

## Usage

```text
pi_calc [--digits N[k|M|B]] [--output PATH] [--format txt|json]
        [--guard-digits N] [--threads N]
```

| Option | Default | Meaning |
|---|---|---|
| `--digits` | `1M` | Digits after the decimal point. Accepts `250000`, `10k`, `100M`. |
| `--output` | `pi_<digits>.<format>` | Output file, for example `pi_100M.txt`. |
| `--format` | `txt` | `txt` writes `3.` and the digits; `json` wraps them with metadata. |
| `--guard-digits` | `20` | Extra digits computed and discarded to protect the last ones. |
| `--threads` | all logical cores | Worker threads. |

The program raises its own process priority while it runs, so other programs
get less CPU for those few seconds.

## Checking the output

- `pi_100.txt`, `pi_10k.txt` and `pi_1M.txt` are reference outputs. A fresh run
  at those sizes should be byte-identical.
- `verify.ps1` checks 50-digit windows of any `txt` output against known digits
  of pi, up to position 1,000,000,000:

```powershell
.\verify.ps1 pi_100M.txt
```

## How it works

1. **Series.** The [Chudnovsky formula](https://en.wikipedia.org/wiki/Chudnovsky_algorithm)
   gives about 14.18 digits per term.
2. **Binary splitting.** The terms are summed as three integers P, Q and T by
   [recursively halving the range](https://en.wikipedia.org/wiki/Binary_splitting),
   so nearly all the work is multiplying very large numbers. Sub-ranges run on
   separate threads.
3. **Smaller numbers.** Common prime factors of P and Q are divided out as
   ranges are merged, using a sieve of smallest prime factors, and Q is carried
   without its powers of two.
4. **Multiplication.** Large products use FLINT's
   [small-prime FFT](https://fredrikj.net/blog/2023/04/flint-furnished-with-faster-fft/),
   which is itself multithreaded.
5. **Final value.** `sqrt(10005)` is computed alongside the series. Pi is formed
   as a binary fixed-point number with a Karp-Markstein division.
6. **Decimal conversion.** The binary fraction is split in halves by
   multiplying with powers of ten, with no division, and the halves convert in
   parallel.

Memory comes from [mimalloc](https://github.com/microsoft/mimalloc). A
100-million-digit run peaks at about 2.7 GB.

## Building

The build targets **Windows on x86-64 with AVX2** and is set up for one
specific toolchain. It is not a plain `cargo build`.

You need:

- Rust, with the `stable-x86_64-pc-windows-msvc` toolchain and the
  `x86_64-pc-windows-gnu` target.
- [Git for Windows](https://gitforwindows.org/) in its default location.
- [w64devkit](https://github.com/skeeto/w64devkit) unpacked to
  `.tools/w64devkit`.
- `m4` from [MSYS2](https://packages.msys2.org/packages/m4) unpacked so that
  `.tools/msys-m4/usr/bin/m4.exe` exists.
- The [FLINT 3.6.0 source](https://github.com/flintlib/flint/releases/tag/v3.6.0)
  unpacked to `.tools/flint-src/flint-3.6.0`.

Then, from the repository root:

```powershell
.\build.ps1          # builds GMP (about 7 minutes), then fails to link
sh build_flint.sh    # builds MPFR and FLINT (about 25 minutes); run in Git Bash
.\build.ps1          # links pi_calc.exe
```

The first `build.ps1` is expected to fail at the link step: it produces the
GMP library that `build_flint.sh` needs. Later builds only need `build.ps1`.

The binary lands in `target\x86_64-pc-windows-gnu\release\pi_calc.exe`.

`build.ps1` builds GMP for Zen 4 (`$gmpCpu = "zen4"`). Change it to `x86_64`
for a build that runs on any x86-64 CPU.

## Limits

- Tested up to 100 million digits. Larger runs are not verified, and one
  attempt at 1 billion digits on an earlier version aborted.
- Windows only as it stands.

## Licence

The code of this project is under the [MIT licence](LICENSE).

That does not cover `vendor/gmp-mpfr-sys`, a copy of the
[gmp-mpfr-sys](https://crates.io/crates/gmp-mpfr-sys) crate, which bundles the
GMP, MPFR and MPC sources under the LGPL. Their licence files are kept in that
directory. FLINT (LGPL) and mimalloc (MIT) are fetched at build time and are
not part of this repository.

A built `pi_calc.exe` links GMP, MPFR and FLINT statically, so anyone who
passes the binary on has to meet the LGPL's terms for those libraries as well.
