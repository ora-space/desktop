//! Generic, domain-free building blocks shared by every Ora crate.
//!
//! The crate deliberately depends on no other `ora-*` crate and carries no domain vocabulary,
//! so any crate can consume it without introducing dependency cycles. Heavier optional
//! capabilities such as archive extraction are gated behind Cargo features so path-only
//! consumers stay light.

#[cfg(feature = "archive")]
pub mod archive;
pub mod atomic;
mod git_branch;
#[cfg(feature = "validation")]
pub mod hash;
#[cfg(feature = "validation")]
pub mod html;
#[cfg(feature = "http")]
pub mod http;
pub mod path;
mod slug;
#[cfg(feature = "validation")]
pub mod svg;

pub use git_branch::{GitBranchName, GitBranchNameError};
pub use slug::{Slug, SlugError};
