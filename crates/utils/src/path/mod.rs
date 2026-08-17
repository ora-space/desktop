//! Platform-independent path validation, containment, and lexical normalization.
//!
//! Two relative-path types coexist on purpose: [`PortableRelativePath`] is the lenient parser for
//! wire and configuration input (empty and `.` segments are dropped), while
//! [`StrictRelativePath`] is the strict parser for untrusted archive and package entries (any
//! irregular spelling is rejected and length/depth limits apply). Callers must not blur the two.

mod containment;
mod lexical;
mod portable;
mod strict;

pub use containment::{CanonicalPathRoot, PathContainmentError};
pub use lexical::{canonicalize_longest_existing_prefix, normalize_absolute, normalize_relative};
pub use portable::{PortableRelativePath, PortableRelativePathError};
pub use strict::{RelativePathLimits, StrictRelativePath, StrictRelativePathError};
