//! Release and source-control identity for the current JMAN build.

include!(concat!(env!("OUT_DIR"), "/build_info.rs"));

#[doc(hidden)]
pub mod versioning;
