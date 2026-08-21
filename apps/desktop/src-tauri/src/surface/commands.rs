//! Tauri commands of the surface host: DTO translation only, every decision lives in
//! `SurfaceService`.

use crate::error::CommandError;
use crate::state::DesktopState;
use crate::surface::capabilities::SurfaceCapabilities;
use ora_domain::PluginId;
use ora_surface::{MountTarget, SurfaceInstanceId, SurfaceRecord, SurfaceState};
use serde::{Deserialize, Serialize};
use tauri::{LogicalPosition, LogicalSize, State};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSurfaceRequest {
    /// Canonical `<namespace>/<name>`; a malformed id is rejected during argument parsing.
    plugin_id: PluginId,
    surface_id: String,
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
    pub surface_id: String,
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
            plugin_id: record.definition.id.plugin_id.to_string(),
            surface_id: record.definition.id.surface_id.as_str().to_owned(),
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
    let record = state
        .surfaces
        .open(&request.plugin_id, &request.surface_id, request.target)?;
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
