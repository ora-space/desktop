//! Tauri-free domain layer for plugin-contributed UI surfaces.
//!
//! The crate owns surface identity, definitions, navigation policy, the open/migrate/close state
//! machine, the process-wide registry, workbench asset URLs, and the webview-plugin download
//! pipeline (intent, rule selection, and the managed-download state machine). Every decision is
//! made here as pure data; the desktop host executes the returned effects against real webviews
//! and the shared safe-landing module.

mod assets;
mod definition;
mod downloads;
mod events;
mod ids;
mod navigation;
mod registry;
mod state;

pub use assets::{
    ASSET_SCHEME, AssetRequest, AssetUrlForm, asset_base, asset_content_type, entry_url,
    workbench_csp,
};
pub use definition::{
    InstancePolicy, MountTarget, RemoteSiteDefinition, SurfaceDefinition, SurfaceKind,
    SurfaceSource, WorkbenchDefinition,
};
pub use downloads::{
    DownloadDecision, DownloadIntent, DownloadOutcome, ManagedDownload, SelectedAction,
    select_disposition,
};
pub use events::SurfaceEvent;
pub use ids::{DownloadId, OperationId, SurfaceInstanceId, ViewGeneration, WebviewLabel};
pub use navigation::NavigationPolicy;
pub use registry::{CommandError, CompleteError, OpenError, SurfaceRecord, SurfaceRegistry};
pub use state::{
    StaleCompletion, SurfaceCommand, SurfaceCompletion, SurfaceEffect, SurfaceState, Transition,
    TransitionContext, TransitionError, apply_command, apply_completion,
};
