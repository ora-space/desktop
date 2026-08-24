//! Tauri commands of the surface host: DTO translation only, every decision lives in
//! `SurfaceService`.

use crate::error::CommandError;
use crate::state::DesktopState;
use crate::surface::capabilities::SurfaceCapabilities;
use ora_domain::PluginId;
use ora_surface::{MountTarget, SurfaceInstanceId, SurfaceKind, SurfaceRecord, SurfaceState};
use serde::{Deserialize, Serialize};
use tauri::{LogicalPosition, LogicalSize, State};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSurfaceRequest {
    /// Canonical `<namespace>/<name>`; a malformed id is rejected during argument parsing.
    plugin_id: PluginId,
    target: MountTarget,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceInstanceRequest {
    instance: u64,
}

/// Placeholder rectangle in CSS pixels. `scale` is the frontend's `devicePixelRatio`; Tauri's
/// logical units are CSS pixels already, so it is accepted for diagnostics and not applied.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSurfaceBoundsRequest {
    instance: u64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[allow(dead_code)]
    scale: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSurfaceVisibleRequest {
    instance: u64,
    visible: bool,
}

/// Frontend projection of one live instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceRecordDto {
    pub instance: u64,
    pub plugin_id: String,
    pub kind: SurfaceKind,
    pub title: String,
    pub target: MountTarget,
    pub state: SurfaceStateDto,
}

/// Coarse lifecycle the frontend renders; `Embedded`/`Windowed` both read as `open`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceStateDto {
    Opening,
    Open,
    Migrating,
    Closing,
    Failed,
}

impl From<&SurfaceRecord> for SurfaceRecordDto {
    fn from(record: &SurfaceRecord) -> Self {
        let (target, state) = match &record.state {
            SurfaceState::Opening { target, .. } => (*target, SurfaceStateDto::Opening),
            SurfaceState::Embedded { .. } => (MountTarget::Embedded, SurfaceStateDto::Open),
            SurfaceState::Windowed { .. } => (MountTarget::Windowed, SurfaceStateDto::Open),
            SurfaceState::Migrating { to, .. } => (*to, SurfaceStateDto::Migrating),
            // A closing instance that never mounted reports the windowed default; the frontend
            // only uses the target of open instances.
            SurfaceState::Closing { from, .. } => (
                from.unwrap_or(MountTarget::Windowed),
                SurfaceStateDto::Closing,
            ),
            SurfaceState::Failed { target, .. } => (*target, SurfaceStateDto::Failed),
        };
        Self {
            instance: record.instance.value(),
            plugin_id: record.definition.plugin_id.to_string(),
            kind: record.definition.kind(),
            title: record.definition.title.clone(),
            target,
            state,
        }
    }
}

#[tauri::command]
pub async fn surface_capabilities(
    state: State<'_, DesktopState>,
) -> Result<SurfaceCapabilities, CommandError> {
    Ok(state.surfaces.capabilities())
}

#[tauri::command]
pub async fn surface_list(
    state: State<'_, DesktopState>,
) -> Result<Vec<SurfaceRecordDto>, CommandError> {
    Ok(state
        .surfaces
        .list()
        .iter()
        .map(SurfaceRecordDto::from)
        .collect())
}

#[tauri::command]
pub async fn surface_open(
    state: State<'_, DesktopState>,
    request: OpenSurfaceRequest,
) -> Result<SurfaceRecordDto, CommandError> {
    let record = state.surfaces.open(&request.plugin_id, request.target)?;
    Ok(SurfaceRecordDto::from(&record))
}

#[tauri::command]
pub async fn surface_close(
    state: State<'_, DesktopState>,
    request: SurfaceInstanceRequest,
) -> Result<(), CommandError> {
    Ok(state
        .surfaces
        .close(SurfaceInstanceId::new(request.instance))?)
}

#[tauri::command]
pub async fn surface_set_bounds(
    state: State<'_, DesktopState>,
    request: SetSurfaceBoundsRequest,
) -> Result<(), CommandError> {
    Ok(state.surfaces.set_bounds(
        SurfaceInstanceId::new(request.instance),
        LogicalPosition::new(request.x, request.y),
        LogicalSize::new(request.width, request.height),
    )?)
}

#[tauri::command]
pub async fn surface_set_visible(
    state: State<'_, DesktopState>,
    request: SetSurfaceVisibleRequest,
) -> Result<(), CommandError> {
    Ok(state
        .surfaces
        .set_visible(SurfaceInstanceId::new(request.instance), request.visible)?)
}

#[tauri::command]
pub async fn surface_popout(
    state: State<'_, DesktopState>,
    request: SurfaceInstanceRequest,
) -> Result<(), CommandError> {
    Ok(state
        .surfaces
        .popout(SurfaceInstanceId::new(request.instance))?)
}

#[tauri::command]
pub async fn surface_dock(
    state: State<'_, DesktopState>,
    request: SurfaceInstanceRequest,
) -> Result<(), CommandError> {
    Ok(state
        .surfaces
        .dock(SurfaceInstanceId::new(request.instance))?)
}

#[tauri::command]
pub async fn surface_reload(
    state: State<'_, DesktopState>,
    request: SurfaceInstanceRequest,
) -> Result<(), CommandError> {
    Ok(state
        .surfaces
        .reload(SurfaceInstanceId::new(request.instance))?)
}

/// Names a host download action the trusted main webview may pick for a webview-plugin download.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDownloadRequest {
    download_id: u64,
    action: String,
    /// Absolute destination path chosen through the host save dialog; required for `save_as`.
    #[serde(default)]
    destination: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardDownloadRequest {
    download_id: u64,
}

/// The result the frontend acts on: for `import_skill`, the id of the prepared import session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDownloadOutcome {
    pub action: String,
    pub import_session_id: Option<String>,
}

#[tauri::command]
pub async fn surface_resolve_download(
    state: State<'_, DesktopState>,
    request: ResolveDownloadRequest,
) -> Result<ResolveDownloadOutcome, CommandError> {
    state
        .resolve_surface_download(request.download_id, &request.action, request.destination)
        .await
}

#[tauri::command]
pub async fn surface_discard_download(
    state: State<'_, DesktopState>,
    request: DiscardDownloadRequest,
) -> Result<(), CommandError> {
    // A download that is already gone (double dismiss) is not an error the frontend must handle.
    let _ = state.surfaces.discard_download(request.download_id);
    Ok(())
}
