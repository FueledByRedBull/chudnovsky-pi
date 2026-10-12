use std::fs;
use std::path::Path;
use std::process::Command;

/// Runs the binary and checks its output against the first `count` digits of
/// the 1M-digit reference, which is "3." and the digits, then a newline.
fn check(digits: &str, count: usize, threads: &str) {
    let reference = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("pi_1M.txt")).unwrap();
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pi_{digits}_{threads}.txt"));

    let status = Command::new(env!("CARGO_BIN_EXE_pi_calc"))
        .args(["--digits", digits, "--threads", threads, "--normal-priority", "--output"])
        .arg(&output)
        .output()
        .unwrap()
        .status;
    assert!(status.success(), "pi_calc failed: {status}");

    let mut expected = reference[..2 + count].to_vec();
    expected.push(b'\n');
    assert!(fs::read(&output).unwrap() == expected, "{digits} digits differ from the reference");
    fs::remove_file(output).unwrap();
}

#[test]
fn matches_reference_digits() {
    check("100k", 100_000, "2");
    check("12345", 12_345, "1");
    // Above about 631k digits the Newton reciprocal and the approximate
    // division replace GMP's division.
    check("700k", 700_000, "1");
    check("1M", 1_000_000, "3");
}
