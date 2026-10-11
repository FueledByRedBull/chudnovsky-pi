//! FLINT 3 as the arithmetic backend. Its AVX2 FFT multiply, Newton square root
//! and radix conversion take over from GMP for large operands. Numbers stay in
//! `rug::Integer`; FLINT reads and writes their limbs directly.

use gmp_mpfr_sys::gmp;
use libmimalloc_sys as mi;
use rug::{Complete, Integer};
use std::ffi::{c_char, c_int, c_long, c_void, CStr};
use std::ptr;

pub const NAME: &str = "FLINT 3 over GMP";

/// Below this FLINT has no edge over GMP, which needs no buffer handling.
const MIN_LIMBS: usize = 256;

/// `fmpz`: a small value, or a tagged pointer to a GMP integer.
type Fmpz = i64;
/// `arb_struct`: a 32-byte midpoint (`arf_struct`) followed by a 16-byte radius.
type Arb = [u64; 6];
const ARF_RND_DOWN: c_int = 0;

// Sizes follow the C declarations: `mp_size_t` is a 32-bit long on Windows,
// FLINT's own `slong` is always 64 bits.
extern "C" {
    // Part of mimalloc, though libmimalloc-sys does not declare it.
    fn mi_calloc(count: usize, size: usize) -> *mut c_void;
    fn fflush(stream: *mut c_void) -> c_int;
    fn abort() -> !;
    fn flint_set_abort(func: extern "C" fn());
    fn __flint_set_all_memory_functions(
        alloc: unsafe extern "C" fn(usize) -> *mut c_void,
        calloc: unsafe extern "C" fn(usize, usize) -> *mut c_void,
        realloc: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void,
        free: unsafe extern "C" fn(*mut c_void),
        aligned_alloc: unsafe extern "C" fn(usize, usize) -> *mut c_void,
        aligned_free: unsafe extern "C" fn(*mut c_void),
    );
    fn flint_set_num_threads(threads: c_int);
    fn flint_reset_num_workers(workers: c_int);
    fn flint_cleanup();
    fn flint_free(ptr: *mut c_void);
    fn _flint_mpn_mul(r: *mut u64, x: *const u64, xn: c_long, y: *const u64, yn: c_long) -> u64;
    fn flint_mpn_get_str(
        res: *mut c_char,
        base: c_int,
        x: *const u64,
        xn: c_long,
        negative: c_int,
    ) -> *mut c_char;
    fn fmpz_set_mpz(f: *mut Fmpz, x: *const gmp::mpz_t);
    fn fmpz_get_mpz(x: *mut gmp::mpz_t, f: *const Fmpz);
    fn fmpz_clear(f: *mut Fmpz);
    fn fmpz_divexact(f: *mut Fmpz, g: *const Fmpz, h: *const Fmpz);
    fn arb_init(x: *mut Arb);
    fn arb_clear(x: *mut Arb);
    fn arb_sqrt_ui(z: *mut Arb, x: u64, prec: i64);
    fn arb_mul_2exp_si(y: *mut Arb, x: *const Arb, e: i64);
    fn arf_get_fmpz(z: *mut Fmpz, x: *const Arb, rnd: c_int) -> c_int;
}

/// FLINT prints its error to C's stdout, which is buffered when redirected and
/// would be lost in the abort.
extern "C" fn flush_and_abort() {
    unsafe {
        fflush(ptr::null_mut());
        abort()
    }
}

// GMP expects its allocator never to return null.
fn checked(ptr: *mut c_void) -> *mut c_void {
    if ptr.is_null() {
        eprintln!("out of memory");
        unsafe { abort() }
    }
    ptr
}
extern "C" fn gmp_alloc(size: usize) -> *mut c_void {
    checked(unsafe { mi::mi_malloc(size) })
}
extern "C" fn gmp_realloc(ptr: *mut c_void, _old: usize, new: usize) -> *mut c_void {
    checked(unsafe { mi::mi_realloc(ptr, new) })
}
extern "C" fn gmp_free(ptr: *mut c_void, _size: usize) {
    unsafe { mi::mi_free(ptr) }
}
unsafe extern "C" fn aligned_alloc(alignment: usize, size: usize) -> *mut c_void {
    mi::mi_malloc_aligned(size, alignment)
}

/// Routes GMP's and FLINT's allocations to mimalloc and starts FLINT's thread
/// pool. Call once, before anything else uses GMP or FLINT.
pub fn start(threads: usize) {
    unsafe {
        gmp::set_memory_functions(Some(gmp_alloc), Some(gmp_realloc), Some(gmp_free));
        __flint_set_all_memory_functions(
            mi::mi_malloc,
            mi_calloc,
            mi::mi_realloc,
            mi::mi_free,
            aligned_alloc,
            mi::mi_free,
        );
        flint_set_abort(flush_and_abort);
        flint_set_num_threads(threads as c_int)
    }
}

/// Frees the calling thread's FFT tables, which FLINT otherwise keeps after
/// the thread is gone. Call as a spawned thread finishes.
pub fn end_thread() {
    unsafe { flint_cleanup() }
}

/// Lets the calling thread borrow up to `threads - 1` workers from the pool.
/// The setting is per thread, so every entry point sets it for itself.
fn use_threads(threads: usize) {
    unsafe { flint_reset_num_workers(threads as c_int - 1) }
}

/// `a * b` on up to `threads` threads (FLINT uses at most eight per product).
pub fn mul(a: &Integer, b: &Integer, threads: usize) -> Integer {
    let (mut x, mut y) = (a.as_limbs(), b.as_limbs());
    if x.len() < y.len() {
        (x, y) = (y, x);
    }
    if y.len() < MIN_LIMBS {
        return (a * b).complete();
    }

    let len = x.len() + y.len();
    let mut product = Integer::new();
    use_threads(threads);
    // SAFETY: the destination is grown to `len` limbs and overlaps neither
    // operand; `x` is the longer operand and `y` is not empty, as FLINT requires.
    unsafe {
        let raw = product.as_raw_mut();
        gmp::_mpz_realloc(raw, len as gmp::size_t);
        let top = _flint_mpn_mul(
            (*raw).d.as_ptr(),
            x.as_ptr(),
            x.len() as c_long,
            y.as_ptr(),
            y.len() as c_long,
        );
        let len = (len - usize::from(top == 0)) as c_int;
        (*raw).size = if a.is_negative() != b.is_negative() {
            -len
        } else {
            len
        };
    }
    product
}

/// Divides `n` by `d`, which must divide it exactly. FLINT switches to Newton
/// division on its FFT for large operands.
pub fn div_exact(n: &mut Integer, d: &Integer) {
    if d.as_limbs().len() < MIN_LIMBS {
        n.div_exact_mut(d);
        return;
    }
    use_threads(1);
    // SAFETY: both fmpz values are set before use and cleared after; FLINT
    // allows the quotient to alias the dividend.
    unsafe {
        let (mut dividend, mut divisor): (Fmpz, Fmpz) = (0, 0);
        let dividend = ptr::addr_of_mut!(dividend);
        fmpz_set_mpz(dividend, n.as_raw());
        fmpz_set_mpz(&mut divisor, d.as_raw());
        fmpz_divexact(dividend, dividend, &divisor);
        fmpz_get_mpz(n.as_raw_mut(), dividend);
        fmpz_clear(dividend);
        fmpz_clear(&mut divisor);
    }
}

/// floor(sqrt(10005) * 2^(64 * limbs)), give or take a unit, by FLINT's Newton
/// iteration for the square root.
pub fn sqrt_10005(limbs: usize, threads: usize) -> Integer {
    // Covers the integer part of the root with a limb to spare.
    let prec = 64 * (limbs as i64 + 2);
    let mut root = Integer::new();
    use_threads(threads);
    // SAFETY: `value` and `floor` are initialised before use and cleared after;
    // FLINT allows the output of `arb_mul_2exp_si` to alias its input.
    unsafe {
        let mut value: Arb = [0; 6];
        let value = ptr::addr_of_mut!(value);
        let mut floor: Fmpz = 0;
        arb_init(value);
        arb_sqrt_ui(value, 10_005, prec);
        arb_mul_2exp_si(value, value, 64 * limbs as i64);
        // The midpoint is the first field of an arb.
        arf_get_fmpz(&mut floor, value, ARF_RND_DOWN);
        fmpz_get_mpz(root.as_raw_mut(), &floor);
        fmpz_clear(&mut floor);
        arb_clear(value);
    }
    root
}

/// Decimal digits of a non-negative value.
pub fn to_decimal(value: &Integer) -> String {
    let limbs = value.as_limbs();
    if limbs.len() < MIN_LIMBS {
        return value.to_string_radix(10);
    }
    use_threads(1);
    // SAFETY: FLINT allocates the NUL-terminated result, which is freed here.
    unsafe {
        let text = flint_mpn_get_str(
            ptr::null_mut(),
            10,
            limbs.as_ptr(),
            limbs.len() as c_long,
            0,
        );
        let digits = CStr::from_ptr(text)
            .to_str()
            .expect("decimal digits are ASCII")
            .to_owned();
        flint_free(text.cast());
        digits
    }
}
