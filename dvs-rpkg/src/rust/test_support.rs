use miniextendr_api::miniextendr;

/// Returns 2.5 GiB (2,684,354,560 bytes) as a `u64`.
/// Exercises the Rust → R boundary for large byte counts in testthat tests.
#[miniextendr(noexport)]
pub fn dvs_test_2_5_gib_bytes() -> u64 {
    2_684_354_560u64 // 2.5 * 1024^3
}
