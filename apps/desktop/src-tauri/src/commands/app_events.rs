//! Application event subscriptions at the Desktop stream seam.

use super::stream::StreamStart;
use crate::{error::CommandError, state::DesktopState};
use ora_contracts::WatchAppEventsRequest;
use tauri::State;

/// Attaches the application invalidation source to the common stream lifecycle.
pub(super) async fn start_watch(
    state: State<'_, DesktopState>,
    _request: WatchAppEventsRequest,
    context: StreamStart,
) -> Result<(), CommandError> {
    context
        .events(async { Ok(state.backend.app_events().subscribe()) })
        .await
}
