//! dvsR: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).

use miniextendr_api::{miniextendr, miniextendr_module};

/// @title Hello DVS
/// @description A simple test function to verify the Rust bindings work.
/// @param name A character string with the user's name.
/// @return A greeting string.
/// @examples
/// dvs_hello("World")
/// @export
#[miniextendr]
pub fn dvs_hello(name: &str) -> String {
    format!("Hello, {}! DVS is ready.", name)
}

/// @title DVS Version
/// @description Returns the version of the dvsR Rust backend.
/// @return A character string with the version.
/// @examples
/// dvs_version()
/// @export
#[miniextendr]
pub fn dvs_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

miniextendr_module! {
    mod dvsr;

    fn dvs_hello;
    fn dvs_version;
}
