use crate::state::BinaryResolutionError;
use ora_backend::{BackendBootstrapError, BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::ContractError;
use serde::Serialize;
use thiserror::Error;

/// Reports failures that prevent the Desktop runtime from constructing its managed state.
#[derive(Debug, Error)]
pub enum DesktopBootstrapError {
    #[error("failed to resolve the system application data directory")]
    AppDataDirectory(#[source] tauri::Error),
    #[error("failed to resolve the Ora home directory")]
    OraHomeDirectory(#[source] tauri::Error),
    #[error("invalid ORA_LOG_LEVEL value {value}")]
    InvalidLogLevel { value: String },
    #[error(transparent)]
    Logging(#[from] ora_logging::LoggingInitError),
    #[error("failed to apply the persisted Desktop log level")]
    LoggingReload(#[from] ora_logging::LogLevelReloadError),
    #[error(transparent)]
    Binaries(#[from] BinaryResolutionError),
    #[error("failed to start the process reaper")]
    ProcessReaper(#[source] std::io::Error),
    #[error(transparent)]
    Backend(#[from] BackendBootstrapError),
    #[error("failed to load the persisted Desktop runtime preference")]
    RuntimePreference(#[source] BackendError),
    #[error("failed to initialize Desktop update service")]
    Update(#[source] crate::update::UpdateError),
    #[error("failed to schedule automatic marketplace synchronization")]
    MarketplaceSync(#[source] ora_scheduler::SchedulerError),
}

/// Serializes the transport-neutral contract directly across the Tauri command seam.
///
/// Deliberately has no `From<BackendError>` impl, and one must not be added. Such an impl makes
/// `?` compile inside commands that already own a `RequestLifecycle`, and the conversion has no
/// lifecycle to reach, so it must mint a throwaway one to complete against. That silently breaks
/// both request invariants documented in `docs/runtime-logging.md`: the request records two
/// completion events instead of one (a `failure` under the throwaway id, plus an `abandoned` one
/// at `DEBUG` when the real lifecycle drops unclaimed), and the id handed to the frontend stops
/// matching the request span, so the id the user is asked to quote for support resolves to a
/// record carrying neither the real operation name nor its duration. Requiring an explicit choice
/// between the two constructors below keeps that decision visible in review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CommandError(ContractError);

impl CommandError {
    /// Completes one Tauri request and projects its typed public payload.
    ///
    /// Only for seams that have no ambient lifecycle to complete against, such as managed-state
    /// accessors and surface commands. A command that owns a lifecycle must use
    /// [`Self::from_backend_with_lifecycle`] instead, so the request keeps one id and one
    /// completion event.
    pub fn from_backend(error: BackendError) -> Self {
        let lifecycle = RequestLifecycle::start("tauri_command", &UuidRequestIdGenerator);
        Self::from_backend_with_lifecycle(error, &lifecycle)
    }

    /// Projects a failure through a lifecycle created at the command entry seam.
    pub fn from_backend_with_lifecycle(error: BackendError, lifecycle: &RequestLifecycle) -> Self {
        lifecycle.complete_failure(&error);
        Self(error.contract_error(lifecycle.request_id()))
    }
}
