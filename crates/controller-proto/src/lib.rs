//! Generated client of the Cloud internal control contract (`ora.cloud.internal.v1`). The `.proto`
//! files live in the Cloud repository and are pinned by the `third_party/cloud` submodule;
//! regenerate with `task proto:generate` and never edit `src/gen` by hand.
//!
//! Production code uses only the generated clients. The server modules exist solely behind the
//! `test-server` feature, which tests enable to host an in-memory Cloud over the real contract.
#![allow(clippy::all, clippy::pedantic, clippy::nursery)]

/// `ora.cloud.internal.v1`: lease, execution coordination and control signals as a Controller
/// consumes them. The prost output pulls in the tonic module generated beside it; its server half
/// compiles only with the `test-server` feature.
pub mod v1 {
    include!("gen/ora/cloud/internal/v1/ora.cloud.internal.v1.rs");
}
