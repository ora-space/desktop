//! Domain types and bounded wire messages for process runtime, guardian discovery and helper inspection.

mod containment;
mod guardian;
mod guardian_management;
pub use guardian_management::{
    GuardianHostSession, GuardianManagementOperation, GuardianManagementRejection,
    GuardianManagementReply, GuardianManagementRequest, GuardianRequest,
};
mod guardian_wire;
pub use guardian_wire::{
    GUARDIAN_MAX_FRAME, GUARDIAN_WIRE_VERSION, GuardianAccess, GuardianBootstrap, GuardianChannel,
    GuardianReady, GuardianReadyRequest, decode_guardian_payload, encode_guardian_frame,
};
mod helper;
mod host_run;
pub use host_run::HostRunIntent;
mod host;
pub use host::{
    HOST_WIRE_VERSION, HostCoordination, HostOperation, HostRejection, HostReply, HostRequest,
    HostRunView, HostScopeView,
};
mod output;
mod run;
mod state;

pub use containment::{ContainmentGuarantee, ContainmentRequest};
pub use guardian::{
    GuardianInstanceId, HostBinding, HostInstanceId, InvalidProcessIdentity, RunId,
    ScopeCreationIntent, ScopeId,
};
pub use helper::{HelperOperation, HelperRequest, HelperResponse, HelperStatus};
pub use output::{OutputPolicy, OutputRead, OutputState, OutputStream};
pub use run::{DescendantPolicy, LaunchFact, RunLifetime, RunSnapshot, RunSpec};
pub use state::{
    CleanupEvidence, CleanupState, DirectProcessState, ExitOutcome, ScopeState, StopRequest,
};

mod guardian_run;
pub use guardian_run::{
    GUARDIAN_CAPTURE_LIMIT, GUARDIAN_OUTPUT_CHUNK_LIMIT, GUARDIAN_RUN_LIMIT,
    GuardianHostDisconnect, GuardianRunOperation, GuardianRunRejection, GuardianRunReply,
    GuardianRunRequest, GuardianRunResult,
};
