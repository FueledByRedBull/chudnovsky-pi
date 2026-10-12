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
        [--guard-digits N] [--threads N] [--normal-priority]
```

| Option | Default | Meaning |
|---|---|---|
| `--digits` | `1M` | Digits after the decimal point. Accepts `250000`, `10k`, `100M`. |
| `--output` | `pi_<digits>.<format>` | Output file, for example `pi_100M.txt`. |
| `--format` | `txt` | `txt` writes `3.` and the digits; `json` wraps them with metadata. |
| `--guard-digits` | `20` | Extra digits computed and discarded to protect the last ones. |
| `--threads` | all logical cores | Worker threads. |
| `--normal-priority` | off | Leave the process priority alone. |

On Windows the program raises its own process priority while it runs, so other
programs get less CPU for those few seconds. `--normal-priority` turns that off.

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

## How it got fast

The first version took 290 s for 100 million digits. The changes below took
it to about 5 s. Figures are
for 100 million digits on 16 threads; "split" is the binary-splitting phase,
which the total time follows.

The machine was shared with other programs at 60 to 90% CPU load, and the load
differed from step to step, so compare the two figures inside a row, not
figures from different rows.

| Change | Before -> after |
|---|---|
| GMP built with its Zen 4 assembly instead of with assembly disabled | about 3.2x faster |
| Common prime factors divided out of P and Q while splitting | split 12.8 s -> about 8.5 s |
| Sub-ranges run in parallel down to depth log2(threads) + 1 | split about 8.8 s -> 7.5 s |
| Arithmetic moved from GMP to FLINT 3 and its multithreaded FFT | total 15.4-15.9 s -> 7.5-7.9 s |
| Bottom of the tree summed in 64-term blocks, with no factor bookkeeping inside them | split 5.6-6.1 s -> 4.7-4.9 s |
| Q and T of a single term kept in 128-bit integers | split 3.91 s -> 3.80 s |
| mimalloc for Rust, GMP and FLINT allocations | split 3.80 s -> 3.53 s, total 5.97 s -> 5.65 s |
| Pi held as a binary fixed-point number, converted to decimal by multiplication only | total 5.60 s -> 5.15 s |
| Q carried without its powers of two | split 3.92 s -> 3.60 s, total 5.81 s -> 5.55 s |
| Process priority raised at startup | total 6.39 s -> 5.15 s on a busy machine |
| Sieve of smallest prime factors run in parallel | 0.13-0.19 s -> 0.01 s |
| Karp-Markstein final division | division 0.79 s -> 0.67 s, total 4.93 s -> 4.84 s |

Tried, measured and dropped because they gave nothing:

- `-C target-cpu=native`: no change; the hot code is in GMP and FLINT.
- Square root on 1, 2 or 4 threads: the stage took 4.9, 3.9 and 3.5 s, and the
  total did not move, because the root is not on the critical path.
- Bottom blocks of 256 to 16384 terms instead of 64: equal or slower.
- Exact division through a 2-adic inverse on FLINT's multiply: slower at every
  threshold tried.
- A merge that reuses FFT transforms, written against FLINT's internals: 15 to
  20% faster on mid-sized operands, 30 to 55% slower at the top five levels
  under 16 threads, so no gain overall.
- An allocation-free leaf on fixed stack buffers: single-thread leaf time
  2.15 s -> 1.92 s, nothing at 16 threads.
- Decimal conversion at 5 to 13 levels instead of log2(threads) + 1: no
  difference.

Memory was the obstacle above 400 million digits rather than time. Each thread
kept the FFT buffers of its largest product while the levels below added their
own; releasing them after each level of the decimal conversion took a
400-million-digit run from 9.8 GB to 6.4 GB with no change in time.

## Building

A prebuilt `pi_calc.exe` for any x86-64 CPU with AVX2 is on the
[releases page](https://github.com/FueledByRedBull/chudnovsky-pi/releases).
Building it yourself gives a binary tuned for your CPU.

The build targets **Windows on x86-64 with AVX2** and is set up for one
specific toolchain. It is not a plain `cargo build`.

You need:

- Rust, with the `stable-x86_64-pc-windows-msvc` toolchain and the
  `x86_64-pc-windows-gnu` target.
- [Git for Windows](https://gitforwindows.org/), with `git` on `PATH`.

Then, from the repository root:

```powershell
.\build.ps1
```

The first run downloads [w64devkit](https://github.com/skeeto/w64devkit),
[m4](https://packages.msys2.org/packages/m4), the
[FLINT 3.6.0](https://github.com/flintlib/flint/releases/tag/v3.6.0) source and
the [gmp-mpfr-sys](https://crates.io/crates/gmp-mpfr-sys) crate into `.tools/`,
each checked against a pinned SHA-256. It then builds GMP, MPFR and FLINT, which
takes about half an hour, and finally `pi_calc.exe`. Later runs only rebuild
the Rust code.

By default GMP and FLINT are tuned for Zen 4. `.\build.ps1 -Cpu generic` builds
a portable binary instead: GMP picks its code at run time, though FLINT still
needs AVX2. Other GMP CPU names work the same way as `zen4`. One checkout holds
one CPU choice; to switch, use a fresh checkout.
In one comparison at 100 million digits on a Ryzen 7 7800X3D the portable
binary took 7.81 s against 7.27 s for the tuned one (medians of six runs on 8
threads).

The binary lands in `target\x86_64-pc-windows-gnu\release\pi_calc.exe`.
`.\build.ps1 -Test` also runs the tests, which compare the program's output
with the reference digits in `pi_1M.txt`.

## Limits

- Tested up to 700 million digits (73 s on 8 threads, 11.6 GB peak memory),
  with the digits checked up to the 100 millionth. 1 billion digits has not
  been run: it is expected to need about 16.5 GB of free memory.
- Stops with an error above about 1.29 billion digits, where GMP's 32-bit bit
  counts on Windows run out.
- The build is Windows only. The source no longer depends on Windows, and
  where GMP's bit counts are 64-bit the ceiling above does not apply, but no
  other system has been built or tested.

## Licence

The code of this project is under the [MIT licence](LICENSE).

The GMP and MPFR sources are not in this repository. They come at build
time from the [gmp-mpfr-sys](https://crates.io/crates/gmp-mpfr-sys) crate,
which bundles them under the LGPL. `patches/gmp-mpfr-sys-1.7.1.patch` holds this
project's one change to them, as the LGPL asks to be stated: GMP's
`mpz/realloc.c` (2026) now allows numbers longer than 2^32 bits on Windows, as
runs above 646 million digits need. FLINT (LGPL) and mimalloc (MIT) are
likewise fetched at build time and are not part of this repository.

A built `pi_calc.exe` links GMP, MPFR and FLINT statically, so anyone who
passes the binary on has to meet the LGPL's terms for those libraries as well.
