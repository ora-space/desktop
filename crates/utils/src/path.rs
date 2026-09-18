//! Platform-independent path validation, containment, and lexical normalization.
//!
//! Two relative-path types coexist on purpose: [`PortableRelativePath`] is the lenient parser for
//! wire and configuration input (empty and `.` segments are dropped), while
//! [`StrictRelativePath`] is the strict parser for untrusted archive and package entries (any
//! irregular spelling is rejected and length/depth limits apply). Callers must not blur the two.

mod containment;
#[cfg(unix)]
mod identity;
mod lexical;
mod native_encoding;
mod portable;
mod strict;
#[cfg(unix)]
mod trusted;

pub use containment::{CanonicalPathRoot, PathContainmentError};
#[cfg(unix)]
pub use identity::DirectoryIdentity;
pub use lexical::{canonicalize_longest_existing_prefix, normalize_absolute, normalize_relative};
pub use native_encoding::{deserialize_native_path, serialize_native_path};
pub use portable::{PortableRelativePath, PortableRelativePathError};
pub use strict::{RelativePathLimits, StrictRelativePath, StrictRelativePathError};
#[cfg(unix)]
pub use trusted::{TrustedPathKind, open_private_path, open_trusted_path};

pub(crate) use portable::is_windows_reserved_device_name;
