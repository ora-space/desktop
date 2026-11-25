//! Native command describing how this Desktop installation started.

use crate::error::CommandError;
use crate::state::DesktopState;
use ora_backend::DatabaseOrigin;

/// Reports whether this process created the local database, which separates a fresh installation
/// from an upgrade by a version that kept no record of its own.
#[tauri::command]
pub async fn is_new_installation(
    state: tauri::State<'_, DesktopState>,
) -> Result<bool, CommandError> {
    Ok(state.backend.database_origin() == DatabaseOrigin::Created)
}
