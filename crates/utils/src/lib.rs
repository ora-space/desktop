//! Generic, domain-free building blocks shared by every Ora crate.
//!
//! The crate deliberately depends on no other `ora-*` crate and carries no domain vocabulary,
//! so any crate can consume it without introducing dependency cycles. Heavier optional
//! capabilities such as archive extraction are gated behind Cargo features so path-only
//! consumers stay light.

#[cfg(feature = "archive")]
pub mod archive;
pub mod path;
