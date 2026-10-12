// Links the static libraries that build_flint.sh produces.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let root = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo:rustc-link-search=native={root}/.tools/flint-src/flint-3.6.0");
    println!("cargo:rustc-link-search=native={root}/.tools/flint-deps/lib");
    // Order matters to the linker: FLINT needs MPFR, and both need GMP, which
    // gmp-mpfr-sys adds after these, from the same directory.
    println!("cargo:rustc-link-lib=static=flint");
    println!("cargo:rustc-link-lib=static=mpfr");
    println!("cargo:rustc-link-lib=static=pthread");
}
