//! Syncs marketplace source repositories and builds the lightweight registry index that surfaces
//! available plugins to consumers.

mod entry;
mod error;
mod host;
mod index;
mod logo;
mod readme;
mod source;

pub use entry::RegistryEntry;
pub use error::RegistryError;
pub use host::current_host_target;
pub use index::{RegistryBuild, RegistryIndex, SkippedManifest};
pub use readme::ReadmeReadError;
pub use source::{RegistrySource, RegistrySync};
