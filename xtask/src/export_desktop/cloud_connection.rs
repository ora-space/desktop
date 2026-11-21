//! Generates the native Cloud connection subscription from the route the emitter uses.

#[path = "../../../apps/desktop/src-tauri/bindings/cloud_connection_event.rs"]
mod binding;

use crate::export_contracts::{GENERATED_FILE_HEADER, write_generated_file};
use ora_contracts::{CloudAccount, CloudConnection, CloudSession};
use std::path::Path;

/// Derives the listener and wire samples from the declarations the emitter uses.
pub(super) fn export(staging: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let web = staging.join("apps").join("desktop").join("web");
    let route = serde_json::to_string(binding::CHANGED_EVENT)?;
    write_generated_file(
        &web.join("cloud-connection.generated.ts"),
        &format!(
            "{GENERATED_FILE_HEADER}import {{ listen }} from \"@tauri-apps/api/event\";\nimport type {{ CloudConnection }} from \"@ora/contracts\";\n\n/** Subscribes to the Desktop-owned Cloud connection route; each event carries the whole connection. */\nexport function onCloudConnectionChanged(listener: (connection: CloudConnection) => void): Promise<() => void> {{\n  return listen<CloudConnection>({route}, (event) => listener(event.payload));\n}}\n"
        ),
    )?;
    let payloads = serde_json::to_string(&[
        CloudConnection::NotConfigured,
        CloudConnection::Configured {
            server: "https://cloud.example".to_owned(),
            session: CloudSession::SignedIn {
                account: CloudAccount {
                    id: "u-1".to_owned(),
                    display_name: "Ora".to_owned(),
                },
            },
        },
    ])?;
    write_generated_file(
        &web.join("cloud-connection-fixtures.generated.ts"),
        &format!(
            "{GENERATED_FILE_HEADER}import type {{ CloudConnection }} from \"@ora/contracts\";\n\n// Actual serde output, checked against the generated DTO and consumed by adapter tests.\nexport const cloudConnectionWire = {{ route: {route}, payloads: {payloads} }} as const satisfies {{ route: string; payloads: readonly CloudConnection[] }};\n"
        ),
    )
}
