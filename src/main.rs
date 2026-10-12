use rug::{Complete, Integer};
use std::cmp::Ordering;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::thread;
use std::time::Instant;

// FLINT 3 supplies the large multiply, the square root and the digit conversion.
mod flint;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const A: i64 = 13_591_409;
const B: i64 = 545_140_134;
const C3_OVER_24: u64 = 10_939_058_860_032_000;
const DIGITS_PER_TERM: f64 = 14.181_647_462_725_478;

/// Odd prime factors of C3_OVER_24 = 2^15 * 3^2 * 5^3 * 23^3 * 29^3.
const C3_OVER_24_FACTORS: [(u32, u32); 4] = [(3, 2), (5, 3), (23, 3), (29, 3)];

/// Common factors are only divided out this many levels below the root; nearer
/// the top the divisions cost more than the smaller operands save.
const GCD_MIN_DEPTH: u32 = 4;

/// Terms summed at the bottom of the tree without factor bookkeeping.
const FACTOR_BLOCK: u32 = 64;

/// Divisors shorter than this divide faster with GMP's own division.
const NEWTON_MIN_LIMBS: usize = 1 << 15;

/// Sorted (prime, power) pairs. Powers of two are left out: P is always odd,
/// so they never cancel.
type Factors = Vec<(u32, u32)>;

struct BinarySplitResult {
    p: Integer,
    /// Q without its factors of two, which `q_twos` counts: they are a cheap
    /// shift where T needs them, and Q is about a seventh shorter without.
    q: Integer,
    q_twos: u64,
    t: Integer,
    p_factors: Factors,
    q_factors: Factors,
}

struct Options {
    digits: u32,
    output: PathBuf,
    format: OutputFormat,
    guard_digits: u32,
    threads: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OutputFormat {
    Txt,
    Json,
}

impl OutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            OutputFormat::Txt => "txt",
            OutputFormat::Json => "json",
        }
    }
}

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = f();
    println!("{label}: {:.3}s", start.elapsed().as_secs_f64());
    value
}

/// Body of every thread this program spawns.
fn worker<T>(work: impl FnOnce() -> T) -> T {
    let value = work();
    flint::end_thread();
    value
}

/// Runs both closures, on two threads when `parallel` is set.
fn join<L: Send, R: Send>(
    parallel: bool,
    left: impl FnOnce() -> L + Send,
    right: impl FnOnce() -> R + Send,
) -> (L, R) {
    if !parallel {
        return (left(), right());
    }
    thread::scope(|scope| {
        let left = scope.spawn(|| worker(left));
        let right = right();
        (left.join().expect("worker thread panicked"), right)
    })
}

/// Work started ahead of the point that needs its result.
enum Task<'scope, T> {
    Running(thread::ScopedJoinHandle<'scope, T>),
    Done(T),
}

impl<T> Task<'_, T> {
    fn finish(self) -> T {
        match self {
            Task::Running(handle) => handle.join().expect("worker thread panicked"),
            Task::Done(value) => value,
        }
    }
}

/// Starts `work` on its own thread, or runs it on the spot when `parallel` is unset.
fn start<'scope, T: Send + 'scope>(
    scope: &'scope thread::Scope<'scope, '_>,
    parallel: bool,
    work: impl FnOnce() -> T + Send + 'scope,
) -> Task<'scope, T> {
    if parallel {
        Task::Running(scope.spawn(|| worker(work)))
    } else {
        Task::Done(work())
    }
}

fn pow10(exponent: u32) -> Integer {
    pow5(exponent) << exponent
}

/// Large powers are squared with FLINT's multiply, which GMP's own power does
/// not use: at 100M digits that is 0.5 s instead of 2 s.
fn pow5(exponent: u32) -> Integer {
    if exponent < 1 << 16 {
        return Integer::u_pow_u(5, exponent).complete();
    }
    let half = pow5(exponent / 2);
    let square = flint::mul(&half, &half, 1);
    if exponent & 1 == 1 {
        square * 5u32
    } else {
        square
    }
}

/// Slots sieved at a time: 128 KiB, which stays in a core's own cache.
const SIEVE_BLOCK: usize = 1 << 15;

/// Smallest prime factor of every odd number up to `limit`, indexed by n / 2.
/// Zero marks a prime. The table is sieved in blocks, shared out over
/// `threads` threads.
fn smallest_odd_factors(limit: u64, threads: usize) -> Vec<u32> {
    // Every odd composite up to `limit` has an odd prime factor of at most
    // its square root; trial division finds those few primes.
    let primes: Vec<u64> = (3..)
        .step_by(2)
        .take_while(|prime| prime * prime <= limit)
        .filter(|&n| (3..).step_by(2).take_while(|d| d * d <= n).all(|d| n % d != 0))
        .collect();

    let mut sieve = vec![0u32; (limit / 2 + 1) as usize];
    let share = sieve.len().div_ceil(threads).next_multiple_of(SIEVE_BLOCK);
    thread::scope(|scope| {
        for (part, slots) in sieve.chunks_mut(share).enumerate() {
            let primes = &primes;
            scope.spawn(move || {
                worker(|| {
                    for (block, slots) in slots.chunks_mut(SIEVE_BLOCK).enumerate() {
                        // The odd number of slot i is first + 2 * i.
                        let first = 2 * (part * share + block * SIEVE_BLOCK) as u64 + 1;
                        let end = first + 2 * slots.len() as u64;
                        // Ascending primes, so the first to reach a slot is the smallest.
                        for &prime in primes.iter().take_while(|&&prime| prime * prime < end) {
                            let mut multiple = (prime * prime).max(first.div_ceil(prime) * prime);
                            if multiple % 2 == 0 {
                                multiple += prime;
                            }
                            while multiple < end {
                                let slot = &mut slots[((multiple - first) / 2) as usize];
                                if *slot == 0 {
                                    *slot = prime as u32;
                                }
                                multiple += 2 * prime;
                            }
                        }
                    }
                })
            });
        }
    });
    sieve
}

/// Appends the factorisation of odd `n`, raised to `power`.
fn push_factors(sieve: &[u32], mut n: u64, power: u32, factors: &mut Factors) {
    while n > 1 {
        let prime = match sieve[(n / 2) as usize] {
            0 => n,
            prime => prime as u64,
        };
        let mut exponent = 0;
        while n % prime == 0 {
            n /= prime;
            exponent += power;
        }
        factors.push((prime as u32, exponent));
    }
}

fn multiply_factors(left: &[(u32, u32)], right: &[(u32, u32)]) -> Factors {
    let mut product = Vec::with_capacity(left.len() + right.len());
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        match left[i].0.cmp(&right[j].0) {
            Ordering::Less => {
                product.push(left[i]);
                i += 1;
            }
            Ordering::Greater => {
                product.push(right[j]);
                j += 1;
            }
            Ordering::Equal => {
                product.push((left[i].0, left[i].1 + right[j].1));
                i += 1;
                j += 1;
            }
        }
    }
    product.extend_from_slice(&left[i..]);
    product.extend_from_slice(&right[j..]);
    product
}

/// Expands a non-empty factor list into its integer value.
fn expand_factors(factors: &[(u32, u32)]) -> Integer {
    if let [(prime, power)] = *factors {
        return Integer::u_pow_u(prime, power).complete();
    }
    let (low, high) = factors.split_at(factors.len() / 2);
    flint::mul(&expand_factors(low), &expand_factors(high), 1)
}

/// Divides gcd(p, q) out of both. Scaling the left P and the right Q by the same
/// amount scales the merged P, Q and T alike, so Q / T is unchanged.
fn remove_common_factors(
    p: &mut Integer,
    p_factors: &mut Factors,
    q: &mut Integer,
    q_factors: &mut Factors,
) {
    let mut common = Factors::new();
    let (mut i, mut j) = (0, 0);
    while i < p_factors.len() && j < q_factors.len() {
        match p_factors[i].0.cmp(&q_factors[j].0) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                let power = p_factors[i].1.min(q_factors[j].1);
                p_factors[i].1 -= power;
                q_factors[j].1 -= power;
                common.push((p_factors[i].0, power));
                i += 1;
                j += 1;
            }
        }
    }
    if common.is_empty() {
        return;
    }

    let gcd = expand_factors(&common);
    flint::div_exact(p, &gcd);
    flint::div_exact(q, &gcd);
    p_factors.retain(|factor| factor.1 != 0);
    q_factors.retain(|factor| factor.1 != 0);
}

/// Shifts by a count of twos. GMP's bit counts are 32 bits on Windows.
fn shift_left(value: Integer, twos: u64) -> Integer {
    value << u32::try_from(twos).expect("too many digits for a 32-bit shift")
}

/// P, Q, T and the twos of Q for one term of the series.
fn single_term(a: u32) -> (Integer, Integer, Integer, u64) {
    if a == 0 {
        return (Integer::from(1), Integer::from(1), Integer::from(A), 0);
    }

    let k = a as u64;
    // 72 * k^3 overflows 64 bits beyond about 500,000 terms.
    let p_term = (6 * k - 5) as u128 * (2 * k - 1) as u128 * (6 * k - 1) as u128;

    // Q without its twos: the odd part of k, cubed, times the odd part of
    // C3_OVER_24. The 128-bit products fit up to about 100M digits; beyond
    // that they fall back to big integers.
    let twos = k.trailing_zeros();
    let odd_cubed = ((k >> twos) as u128).pow(3);
    let odd_constant = C3_OVER_24 >> C3_OVER_24.trailing_zeros();
    let q = odd_cubed
        .checked_mul(odd_constant as u128)
        .map_or_else(|| Integer::from(odd_cubed) * odd_constant, Integer::from);

    let linear = (A + B * a as i64) as u128;
    let mut t = p_term
        .checked_mul(linear)
        .map_or_else(|| Integer::from(p_term) * linear, Integer::from);
    if a & 1 != 0 {
        t = -t;
    }
    let q_twos = 3 * twos + C3_OVER_24.trailing_zeros();
    (Integer::from(p_term), q, t, q_twos as u64)
}

/// P, Q, T and the twos of Q for terms a..b, for the few terms at the bottom
/// of the tree.
fn small_split(a: u32, b: u32) -> (Integer, Integer, Integer, u64) {
    if b - a == 1 {
        return single_term(a);
    }
    let midpoint = a + (b - a) / 2;
    let (left_p, left_q, left_t, left_twos) = small_split(a, midpoint);
    let (right_p, right_q, right_t, right_twos) = small_split(midpoint, b);
    let t = shift_left(left_t * &right_q, right_twos) + &left_p * right_t;
    (
        left_p * right_p,
        left_q * right_q,
        t,
        left_twos + right_twos,
    )
}

/// Factors of P and of Q over terms a..b.
fn block_factors(a: u32, b: u32, sieve: &[u32]) -> (Factors, Factors) {
    let (mut p_factors, mut q_factors) = (Factors::new(), Factors::new());
    for k in (a.max(1) as u64)..(b as u64) {
        for part in [6 * k - 5, 2 * k - 1, 6 * k - 1] {
            push_factors(sieve, part, 1, &mut p_factors);
        }
        push_factors(sieve, k >> k.trailing_zeros(), 3, &mut q_factors);
        q_factors.extend_from_slice(&C3_OVER_24_FACTORS);
    }
    (collect_factors(p_factors), collect_factors(q_factors))
}

/// Sorts a factor list and adds up the powers of repeated primes.
fn collect_factors(mut factors: Factors) -> Factors {
    factors.sort_unstable();
    factors.dedup_by(|next, kept| {
        let same = next.0 == kept.0;
        if same {
            kept.1 += next.1;
        }
        same
    });
    factors
}

/// Sums terms a..b. `need_p` is false along the right edge of the tree, where
/// no later merge reads P.
fn binary_split(
    a: u32,
    b: u32,
    depth: u32,
    need_p: bool,
    parallel_depth: u32,
    threads: usize,
    sieve: &[u32],
) -> BinarySplitResult {
    // Common factors between blocks this small are not worth dividing out, so a
    // block is built plainly and its factors are listed once for the levels above.
    if b - a <= FACTOR_BLOCK {
        let (p, q, t, q_twos) = small_split(a, b);
        let (p_factors, q_factors) = block_factors(a, b, sieve);
        return BinarySplitResult {
            p,
            q,
            q_twos,
            t,
            p_factors,
            q_factors,
        };
    }

    let midpoint = a + (b - a) / 2;
    let parallel = depth < parallel_depth;
    let split =
        |a, b, need_p| binary_split(a, b, depth + 1, need_p, parallel_depth, threads, sieve);
    let (mut left, mut right) = join(
        parallel,
        || split(a, midpoint, true),
        || split(midpoint, b, need_p),
    );

    if depth >= GCD_MIN_DEPTH {
        remove_common_factors(
            &mut left.p,
            &mut left.p_factors,
            &mut right.q,
            &mut right.q_factors,
        );
    }

    // FLINT's FFT is threaded itself, so the products run one at a time, each
    // with this merge's whole share of the threads.
    let product_threads = (threads >> depth).max(1);
    let mul = |x: &Integer, y: &Integer| flint::mul(x, y, product_threads);
    let p = need_p.then(|| mul(&left.p, &right.p));
    let q = mul(&left.q, &right.q);
    let t = shift_left(mul(&left.t, &right.q), right.q_twos) + mul(&left.p, &right.t);

    BinarySplitResult {
        p: p.unwrap_or_default(),
        q,
        q_twos: left.q_twos + right.q_twos,
        t,
        p_factors: multiply_factors(&left.p_factors, &right.p_factors),
        q_factors: multiply_factors(&left.q_factors, &right.q_factors),
    }
}

// The two helpers below move whole limbs of a magnitude. Shifting by a bit count
// instead would overflow GMP's 32-bit bit counts on Windows past 4 billion bits.

/// |value| / 2^(64 * limbs), rounded down.
fn drop_low_limbs(value: &Integer, limbs: usize) -> Integer {
    flint::from_limbs(0, value.as_limbs().get(limbs..).unwrap_or_default())
}

/// |value| * 2^(64 * limbs).
fn raise_limbs(value: &Integer, limbs: usize) -> Integer {
    flint::from_limbs(limbs, value.as_limbs())
}

/// 2^(128 * m) / d for a positive m-limb `d`, give or take a few units. Newton's
/// iteration: the reciprocal of the top half of `d` is right to half the limbs,
/// and one correction step doubles that. Unlike GMP's division, the work is all
/// multiplication, so it spreads over `threads` threads.
fn reciprocal(d: &Integer, threads: usize) -> Integer {
    let m = d.as_limbs().len();
    if m < NEWTON_MIN_LIMBS {
        return raise_limbs(&Integer::from(1), 2 * m) / d;
    }

    let h = m / 2 + 2;
    let estimate = reciprocal(&drop_low_limbs(d, m - h), threads);

    // d * estimate would be exactly 2^(64 * (m + h)) for a perfect estimate.
    let error = raise_limbs(&Integer::from(1), m + h) - flint::mul(d, &estimate, threads);
    let correction = drop_low_limbs(
        &flint::mul(&estimate, &drop_low_limbs(&error, m - h), threads),
        3 * h - m,
    );
    let estimate = raise_limbs(&estimate, m - h);
    if error.is_negative() {
        estimate - correction
    } else {
        estimate + correction
    }
}

/// Limbs at the top of an m-limb divisor that `div_approx` wants the
/// reciprocal of: a little over half, so that one correction step completes
/// the quotient.
fn reciprocal_limbs(m: usize) -> usize {
    if m < NEWTON_MIN_LIMBS {
        m
    } else {
        m / 2 + 3
    }
}

/// `|value| / d`, right in about its top h limbs, where `reciprocal` is that
/// of the top h limbs of the m-limb `d`: 1 / d is about reciprocal / 2^(64 * (m + h)).
fn times_reciprocal(
    value: &Integer,
    reciprocal: &Integer,
    m: usize,
    h: usize,
    threads: usize,
) -> Integer {
    // The limbs below the top h + 3 are too small to matter.
    let cut = value.as_limbs().len().saturating_sub(h + 3);
    let product = flint::mul(&drop_low_limbs(value, cut), reciprocal, threads);
    if cut >= m + h {
        raise_limbs(&product, cut - (m + h))
    } else {
        drop_low_limbs(&product, m + h - cut)
    }
}

/// `n / d` for non-negative `n` and positive `d`, give or take a few units.
/// `reciprocal` is `reciprocal()` of the top `reciprocal_limbs` limbs of `d`,
/// and the quotient must not be longer than `d`. After Karp and Markstein:
/// the half-length reciprocal gives the top half of the quotient, and applied
/// to what that leaves of `n` it gives the rest. That takes a product of half
/// the length where a full reciprocal takes a full one.
fn div_approx(n: &Integer, d: &Integer, reciprocal: &Integer, threads: usize) -> Integer {
    let m = d.as_limbs().len();
    let h = reciprocal_limbs(m);
    let estimate = times_reciprocal(n, reciprocal, m, h, threads);
    if h == m {
        return estimate;
    }

    let low = estimate.as_limbs().len().saturating_sub(h + 2);
    let top = drop_low_limbs(&estimate, low);
    drop(estimate);
    let rest = n - raise_limbs(&flint::mul(d, &top, threads), low);
    let correction = times_reciprocal(&rest, reciprocal, m, h, threads);
    let quotient = raise_limbs(&top, low);
    if rest.is_negative() {
        quotient - correction
    } else {
        quotient + correction
    }
}

/// Sizes of the decimal conversion. The fraction of pi is cut into
/// `2^levels` chunks of `chunk` digits, halving once per level.
#[derive(Clone, Copy)]
struct Conversion {
    chunk: usize,
    levels: u32,
    /// Limbs that hold any number below 10^chunk.
    chunk_limbs: usize,
}

/// Spare limbs carried below every fraction. Each level adds an error of about
/// one unit in the last limb, so a digit can only come out wrong where the
/// digits after a chunk boundary are zero for some 35 places.
const FRACTION_GUARD_LIMBS: usize = 2;

impl Conversion {
    fn new(digits: usize, levels: u32) -> Self {
        let chunk = digits.div_ceil(1 << levels);
        let chunk_limbs = (chunk as f64 * std::f64::consts::LOG2_10 / 64.0) as usize + 1;
        Conversion {
            chunk,
            levels,
            chunk_limbs,
        }
    }

    /// Limbs of a fraction that still has `chunk << level` digits to give.
    /// Doubling exactly with each level keeps 10^(digits of the upper half)
    /// below the limbs the lower half drops.
    fn limbs(self, level: u32) -> usize {
        (self.chunk_limbs << level) + FRACTION_GUARD_LIMBS
    }

    /// 10^(chunk * 2^i) for each i below `levels`, and always 10^chunk.
    fn powers(self, threads: usize) -> Vec<Integer> {
        let mut powers = vec![pow10(self.chunk as u32)];
        for _ in 1..self.levels {
            let last = &powers[powers.len() - 1];
            powers.push(flint::mul(last, last, threads));
        }
        powers
    }
}

/// Writes the first `out.len()` decimal digits of `fraction / 2^(64 * limbs)`,
/// where `limbs` is `conversion.limbs(level)`. The upper half of the digits
/// comes from the top limbs alone; multiplying by the power of ten that shifts
/// those digits out leaves the fraction that gives the lower half. No division
/// is involved, and the halves convert on separate threads.
fn write_fraction(
    fraction: Integer,
    level: u32,
    conversion: Conversion,
    powers: &[Integer],
    out: &mut [u8],
    threads: usize,
) {
    let limbs = conversion.limbs(level);
    if level == 0 {
        let shifted = flint::mul(&fraction, &powers[0], threads);
        let digits = flint::to_decimal(&drop_low_limbs(&shifted, limbs));
        let padding = out.len() - digits.len();
        out[padding..].copy_from_slice(digits.as_bytes());
        return;
    }

    let dropped = limbs - conversion.limbs(level - 1);
    let upper = drop_low_limbs(&fraction, dropped);
    let shifted = flint::mul(&fraction, &powers[level as usize - 1], threads);
    drop(fraction);
    let shifted_limbs = shifted.as_limbs();
    let lower = flint::from_limbs(
        0,
        shifted_limbs
            .get(dropped..limbs.min(shifted_limbs.len()))
            .unwrap_or_default(),
    );
    drop(shifted);
    // This thread's products only get smaller from here, and FLINT would keep
    // the buffers of the largest while every level below adds its own.
    flint::end_thread();

    let (out_upper, out_lower) = out.split_at_mut(out.len() / 2);
    // Twice as many products run at the next level, so each gets half the threads.
    let threads = (threads / 2).max(1);
    join(
        true,
        || write_fraction(upper, level - 1, conversion, powers, out_upper, threads),
        || write_fraction(lower, level - 1, conversion, powers, out_lower, threads),
    );
}

/// The leading 3 followed by `digits_after_decimal` digits, from pi as a
/// fixed-point number with `conversion.limbs(levels)` limbs below the point.
fn fixed_pi_to_digits(
    pi_fixed: Integer,
    digits_after_decimal: u32,
    conversion: Conversion,
    powers: &[Integer],
    threads: usize,
) -> Result<String, Box<dyn Error>> {
    let limbs = conversion.limbs(conversion.levels);
    if pi_fixed.as_limbs().get(limbs..) != Some(&[3]) {
        return Err("internal error: the computed value is not pi".into());
    }
    let fraction = flint::from_limbs(0, &pi_fixed.as_limbs()[..limbs]);
    drop(pi_fixed);

    let mut digits = vec![b'0'; 1 + (conversion.chunk << conversion.levels)];
    digits[0] = b'3';
    write_fraction(
        fraction,
        conversion.levels,
        conversion,
        powers,
        &mut digits[1..],
        threads,
    );
    digits.truncate(1 + digits_after_decimal as usize);
    Ok(String::from_utf8(digits).expect("decimal digits are ASCII"))
}

fn first_chars(digits: &str, count: usize) -> String {
    let mut text = String::with_capacity(count.min(digits.len() + 1));
    if let Some(integer_part) = digits.as_bytes().first() {
        text.push(*integer_part as char);
        text.push('.');
        let fractional_count = count.saturating_sub(2).min(digits.len().saturating_sub(1));
        text.push_str(&digits[1..1 + fractional_count]);
    }
    text
}

fn write_decimal(out: &mut BufWriter<File>, digits: &str) -> Result<(), Box<dyn Error>> {
    let bytes = digits.as_bytes();
    if bytes.is_empty() {
        return Err("scaled Pi digit string is empty".into());
    }

    out.write_all(&bytes[..1])?;
    out.write_all(b".")?;
    out.write_all(&bytes[1..])?;
    out.write_all(b"\n")?;
    Ok(())
}

fn write_output(options: &Options, terms: u32, digits: &str) -> Result<(), Box<dyn Error>> {
    let file = File::create(&options.output)?;
    let mut out = BufWriter::new(file);

    match options.format {
        OutputFormat::Txt => {
            write_decimal(&mut out, digits)?;
        }
        OutputFormat::Json => {
            writeln!(out, "{{")?;
            writeln!(out, "  \"algorithm\": \"Chudnovsky formula\",")?;
            writeln!(out, "  \"acceleration\": \"binary splitting\",")?;
            writeln!(out, "  \"backend\": \"{}\",", flint::NAME)?;
            writeln!(out, "  \"digits_after_decimal\": {},", options.digits)?;
            writeln!(out, "  \"terms\": {terms},")?;
            write!(out, "  \"pi\": \"")?;
            out.write_all(&digits.as_bytes()[..1])?;
            out.write_all(b".")?;
            out.write_all(&digits.as_bytes()[1..])?;
            writeln!(out, "\"")?;
            writeln!(out, "}}")?;
        }
    }

    out.flush()?;
    Ok(())
}

/// Parses a digit count such as `250000`, `10k`, `100M` or `1B`.
fn parse_digits(text: &str) -> Result<u32, Box<dyn Error>> {
    let (number, scale) = match text.as_bytes().last() {
        Some(b'k' | b'K') => (&text[..text.len() - 1], 1_000),
        Some(b'm' | b'M') => (&text[..text.len() - 1], 1_000_000),
        Some(b'b' | b'B') => (&text[..text.len() - 1], 1_000_000_000),
        _ => (text, 1),
    };
    number
        .parse::<u32>()?
        .checked_mul(scale)
        .ok_or_else(|| "--digits is too large".into())
}

/// Short size label for file names: 10000 -> 10k, 100000000 -> 100M, 1000000000 -> 1B.
fn digits_label(digits: u32) -> String {
    for (scale, suffix) in [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "k")] {
        if digits >= scale && digits % scale == 0 {
            return format!("{}{suffix}", digits / scale);
        }
    }
    digits.to_string()
}

fn parse_args() -> Result<Options, Box<dyn Error>> {
    let mut digits = 1_000_000;
    let mut output = None;
    let mut format = OutputFormat::Txt;
    let mut guard_digits = 20;
    let mut threads = thread::available_parallelism().map_or(1, |count| count.get());

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--digits" => {
                digits = parse_digits(&args.next().ok_or("--digits requires a value")?)?;
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a value")?,
                ));
            }
            "--format" => {
                let value = args.next().ok_or("--format requires a value")?;
                format = match value.as_str() {
                    "txt" => OutputFormat::Txt,
                    "json" => OutputFormat::Json,
                    _ => return Err("unsupported format; use txt or json".into()),
                };
            }
            "--guard-digits" => {
                guard_digits = args
                    .next()
                    .ok_or("--guard-digits requires a value")?
                    .parse()?;
            }
            "--threads" => {
                threads = args.next().ok_or("--threads requires a value")?.parse()?;
                if threads == 0 {
                    return Err("--threads must be at least 1".into());
                }
            }
            "--help" | "-h" => {
                println!(
                    "Usage: pi_calc [--digits N[k|M|B]] [--output PATH] [--format txt|json] \
                     [--guard-digits N] [--threads N]\n\
                     The output defaults to pi_<digits>.<format>, for example pi_100M.txt.\n\
                     The most digits this build computes is about 1.29 billion."
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    let output = output.unwrap_or_else(|| {
        PathBuf::from(format!("pi_{}.{}", digits_label(digits), format.as_str()))
    });

    Ok(Options {
        digits,
        output,
        format,
        guard_digits,
        threads,
    })
}

extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn SetPriorityClass(process: *mut std::ffi::c_void, class: u32) -> i32;
}
const HIGH_PRIORITY_CLASS: u32 = 0x80;

fn main() -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let options = parse_args()?;
    // Every core is in use for the whole run, so background programs otherwise
    // take slices of it: 6.4 s against 5.2 s at 100M digits on a busy machine.
    // A refusal only costs that speed.
    // SAFETY: plain Win32 calls on this process's own handle.
    unsafe { SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS) };
    let scaled_digits = options
        .digits
        .checked_add(options.guard_digits)
        .ok_or("digits plus guard digits overflowed u32")?;
    let terms = ((scaled_digits as f64) / DIGITS_PER_TERM).ceil() as u32 + 1;
    // Tasks double with every level. One level more than fills the cores:
    // sub-trees differ in cost, and the spare tasks keep every core busy.
    let parallel_depth = match options.threads {
        1 => 0,
        threads => threads.next_power_of_two().trailing_zeros() + 1,
    };
    let output_path = std::fs::canonicalize(
        options
            .output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new(".")),
    )?
    .join(
        options
            .output
            .file_name()
            .ok_or("output path has no file name")?,
    );

    println!(
        "Digits after decimal: {} ({})",
        options.digits,
        digits_label(options.digits)
    );
    println!("Guard digits: {}", options.guard_digits);
    println!("Chudnovsky terms: {terms}");
    println!("Threads: {}", options.threads);
    println!("Backend: {}", flint::NAME);
    println!("Output format: {}", options.format.as_str());
    println!("Output path: {}\n", output_path.display());

    let threads = options.threads;
    let parallel = threads > 1;
    flint::start(threads);
    // The guard digits are converted too, then left out of the output.
    let conversion = Conversion::new(scaled_digits as usize, parallel_depth);
    // Pi is computed in binary, with this many limbs below the point.
    let limbs = conversion.limbs(conversion.levels);
    // FLINT hands GMP the bit position of the last of them, and GMP's bit
    // counts are 32 bits on Windows. Past that it would go wrong silently.
    if limbs as u64 * 64 > u32::MAX as u64 {
        return Err("too many digits: this build stops at about 1.29 billion".into());
    }
    // Q and T matter only to three limbs beyond that.
    let keep = limbs + 3;

    let (pi_fixed, powers) = thread::scope(|scope| {
        // Neither the square root nor the powers of ten for the conversion
        // depend on the series, so one thread works through both alongside it.
        // The root goes ahead through a channel: the division needs it first.
        let (root_sender, root_receiver) = std::sync::mpsc::channel();
        let powers = start(scope, parallel, move || {
            let root = timed("Square root", || flint::sqrt_10005(limbs, 1));
            root_sender.send(root).expect("the division waits for the root");
            // The root took this thread's largest products; their buffers go before
            // the rest of the run needs the memory.
            flint::end_thread();
            timed("Decimal powers", || conversion.powers(1))
        });
        let split = timed("Binary splitting", || {
            let sieve = smallest_odd_factors(6 * terms as u64, threads);
            let split = binary_split(0, terms, 0, false, parallel_depth, threads, &sieve);
            // Likewise for the buffers of the top merges.
            flint::end_thread();
            split
        });

        let pi_fixed = timed("Final division", || {
            // Both are cut or padded to `keep` limbs, which leaves Q / T as it was
            // and makes T as long as the quotient the division relies on.
            let BinarySplitResult { q, t, q_twos, .. } = split;
            let q = shift_left(q, q_twos);
            let length = t.as_limbs().len();
            let resize = |value: &Integer| {
                if length > keep {
                    drop_low_limbs(value, length - keep)
                } else {
                    raise_limbs(value, keep - length)
                }
            };
            let (q, t) = (resize(&q), resize(&t));

            // The reciprocal needs only the top of T, so it starts before the
            // square root is in.
            let t_reciprocal = start(scope, parallel, move || {
                let low = t.as_limbs().len() - reciprocal_limbs(t.as_limbs().len());
                let t_reciprocal = reciprocal(&drop_low_limbs(&t, low), threads);
                (t, t_reciprocal)
            });
            let root = root_receiver.recv().expect("the square root thread stopped");
            let numerator = flint::mul(&q, &root, threads) * 426_880;
            let (t, t_reciprocal) = t_reciprocal.finish();
            div_approx(&numerator, &t, &t_reciprocal, threads)
        });
        (pi_fixed, powers.finish())
    });

    let pi_digits = timed("Decimal conversion", || {
        fixed_pi_to_digits(pi_fixed, options.digits, conversion, &powers, threads)
    })?;

    timed("File write", || write_output(&options, terms, &pi_digits))?;

    println!("Total: {:.3}s", started.elapsed().as_secs_f64());
    println!("\nFirst 80 chars: {}", first_chars(&pi_digits, 80));
    println!("Completed successfully.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sieve_gives_smallest_odd_factor() {
        for threads in [1, 3, 16] {
            let limit = 300_001;
            let sieve = smallest_odd_factors(limit, threads);
            for n in (3..=limit).step_by(2) {
                let smallest = (3..).step_by(2).take_while(|d| d * d <= n).find(|d| n % d == 0);
                assert_eq!(sieve[(n / 2) as usize] as u64, smallest.unwrap_or(0), "n = {n}");
            }
        }
    }
}
