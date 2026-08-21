//! Tauri-free domain layer for plugin-contributed UI surfaces.
//!
//! The crate owns surface identity, definitions, navigation policy, the open/migrate/close state
//! machine, the process-wide registry, and download reservations. Every decision is made here as
//! pure data; the desktop host executes the returned [`SurfaceEffect`]s against real webviews.

mod definition;
mod downloads;
mod events;
mod ids;
mod navigation;
mod panel;
mod registry;
mod state;

pub use definition::{
    MountTarget, PanelDefinition, RemoteSiteDefinition, SurfaceDefinition, SurfaceSource,
};
pub use downloads::{
    CompletedDownload, DownloadAcceptance, DownloadClock, DownloadCoordinator, DownloadFinish,
    DownloadId, DownloadStatus, LocalDownloadClock, RejectReason, Reservation,
};
pub use events::SurfaceEvent;
pub use ids::{OperationId, SurfaceDefinitionId, SurfaceInstanceId, ViewGeneration, WebviewLabel};
pub use navigation::NavigationPolicy;
pub use panel::{
    PANEL_SCHEME, PanelAssetRequest, PanelUrlForm, panel_asset_base, panel_content_type, panel_csp,
    panel_entry_url,
};
pub use registry::{CommandError, CompleteError, OpenError, SurfaceRecord, SurfaceRegistry};
pub use state::{
    StaleCompletion, SurfaceCommand, SurfaceCompletion, SurfaceEffect, SurfaceState, Transition,
    TransitionContext, TransitionError, apply_command, apply_completion,
};
