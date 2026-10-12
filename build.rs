// Links the static libraries that build_flint.sh produces.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let root = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo:rustc-link-search=native={root}/.tools/flint-src/flint-3.6.0");
    println!("cargo:rustc-link-search=native={root}/.tools/flint-deps/lib");
    // Order matters to the linker: FLINT needs MPFR, and both need GMP.
    println!("cargo:rustc-link-lib=static=flint");
    println!("cargo:rustc-link-lib=static=mpfr");
    // The libgmp.a that gmp-mpfr-sys adds after these is missing objects (see
    // build_flint.sh); this complete copy comes first, so the linker takes
    // everything from it.
    println!("cargo:rustc-link-lib=static=gmpfull");
    println!("cargo:rustc-link-lib=static=pthread");
}
