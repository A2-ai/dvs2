//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

miniextendr_api::miniextendr_module! {
    mod dvs;
}
