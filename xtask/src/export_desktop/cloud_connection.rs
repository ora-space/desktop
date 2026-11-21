//! Generates the native Cloud connection subscription from the route the emitter uses.

#[path = "../../../apps/desktop/src-tauri/bindings/cloud_connection_event.rs"]
mod binding;

use crate::export_contracts::{GENERATED_FILE_HEADER, write_generated_file};
use std::path::Path;

/// Derives the listener from the same route constant the host emits on.
pub(super) fn export(staging: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let web = staging.join("apps").join("desktop").join("web");
    let route = serde_json::to_string(binding::CHANGED_EVENT)?;
    write_generated_file(
        &web.join("cloud-connection.generated.ts"),
        &format!(
            "{GENERATED_FILE_HEADER}import {{ listen }} from \"@tauri-apps/api/event\";\nimport type {{ CloudConnection }} from \"@ora/contracts\";\n\n/** Subscribes to the Desktop-owned Cloud connection route; each event carries the whole connection. */\nexport function onCloudConnectionChanged(listener: (connection: CloudConnection) => void): Promise<() => void> {{\n  return listen<CloudConnection>({route}, (event) => listener(event.payload));\n}}\n"
        ),
    )
}
