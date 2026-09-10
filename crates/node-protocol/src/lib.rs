//! Shared wire contract between Ora Controller and execution Nodes.
//!
//! The crate owns typed protocol messages and their binary framing. It deliberately has no
//! transport, persistence, filesystem, Git, or Ora application-domain dependencies.

mod domain;
mod frame;
mod identity;
mod message;

pub use domain::*;
pub use frame::*;
pub use identity::*;
pub use message::*;
