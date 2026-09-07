//! Joins public operations with Desktop-owned handlers and explicit command grants.

use crate::desktop_bindings::{Binding, Permission, bindings};
use crate::export_contracts::{GENERATED_FILE_HEADER, write_generated_file};
use crate::frontend::{FrontendEndpoint, FrontendResponseMode};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Carries the validated join so renderers cannot publish a partially bound catalog.
struct DesktopCatalog<'a> {
    commands: BTreeMap<&'static str, (&'static str, Permission)>,
    unary: BTreeMap<&'static str, &'static str>,
    streams: BTreeMap<&'static str, (&'a FrontendEndpoint, &'static str)>,
}

/// Emits private routing and authorization without leaking adapter metadata into the public SDK.
pub(crate) fn export(
    staging: &Path,
    endpoints: &[FrontendEndpoint],
) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = validate(endpoints, &bindings())?;
    let desktop = staging.join("apps").join("desktop");
    let tauri = desktop.join("src-tauri");
    write_generated_file(
        &desktop.join("web").join("tauri-bindings.generated.ts"),
        &render_transport(&catalog),
    )?;
    write_generated_file(
        &tauri.join("src").join("app_commands.rs"),
        &render_commands(&catalog),
    )?;
    write_generated_file(
        &tauri.join("src").join("commands").join("stream_routes.rs"),
        &render_stream_routes(&catalog),
    )?;
    for (filename, permission) in [
        ("main-commands.toml", Permission::MainWebview),
        (
            "plugin-webview-commands.toml",
            Permission::MainAndPluginWebviews,
        ),
    ] {
        write_generated_file(
            &tauri.join("permissions").join(filename),
            &render_permission(&catalog, permission),
        )?;
    }
    Ok(())
}

/// Rejects missing, duplicate, unknown, and mode-incompatible bindings before producing artifacts.
fn validate<'a>(
    endpoints: &'a [FrontendEndpoint],
    bindings: &[Binding],
) -> Result<DesktopCatalog<'a>, String> {
    let endpoint_by_name = endpoints
        .iter()
        .map(|endpoint| (endpoint.operation_name, endpoint))
        .collect::<BTreeMap<_, _>>();
    if endpoint_by_name.len() != endpoints.len() {
        return Err("duplicate operation in the public catalog".to_string());
    }
    let mut catalog = DesktopCatalog {
        commands: BTreeMap::new(),
        unary: BTreeMap::new(),
        streams: BTreeMap::new(),
    };
    let mut bound_operations = BTreeSet::new();
    for binding in bindings {
        let (operation, mode) = match *binding {
            Binding::Unary { operation, .. } => (operation, FrontendResponseMode::Unary),
            Binding::Stream { operation, .. } => (operation, FrontendResponseMode::Stream),
            Binding::Native {
                handler,
                permission,
            } => {
                insert_command(&mut catalog, handler, permission)?;
                continue;
            }
        };
        let endpoint = endpoint_by_name
            .get(operation)
            .ok_or_else(|| format!("Desktop binding refers to unknown operation: {operation}"))?;
        if endpoint.response_mode != mode {
            return Err(format!(
                "Desktop binding mode disagrees with operation: {operation}"
            ));
        }
        if !bound_operations.insert(operation) {
            return Err(format!("duplicate Desktop operation binding: {operation}"));
        }
        match *binding {
            Binding::Unary {
                handler,
                permission,
                ..
            } => {
                let command = insert_command(&mut catalog, handler, permission)?;
                catalog.unary.insert(operation, command);
            }
            Binding::Stream { handler, .. } => {
                catalog.streams.insert(operation, (endpoint, handler));
            }
            Binding::Native { .. } => unreachable!("native commands are handled above"),
        }
    }
    let missing = endpoint_by_name
        .keys()
        .filter(|operation| !bound_operations.contains(*operation))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!("missing Desktop bindings: {}", missing.join(", ")));
    }
    // Streams must remain reachable when native bindings change independently of the SDK catalog.
    if !catalog.streams.is_empty() {
        for command in ["stream_contract", "cancel_contract_stream"] {
            if !catalog.commands.contains_key(command) {
                return Err(format!("missing shared stream command: {command}"));
            }
        }
    }
    Ok(catalog)
}

/// Enforces Tauri's flat command-name uniqueness across differently owned Rust modules.
fn insert_command(
    catalog: &mut DesktopCatalog<'_>,
    handler: &'static str,
    permission: Permission,
) -> Result<&'static str, String> {
    let command = handler.rsplit("::").next().unwrap_or(handler);
    if command.is_empty() || !handler.contains("::") {
        return Err(format!(
            "Desktop handler must be a qualified Rust path: {handler}"
        ));
    }
    if catalog
        .commands
        .insert(command, (handler, permission))
        .is_some()
    {
        return Err(format!("duplicate Desktop command: {command}"));
    }
    Ok(command)
}

/// Emits the registry consumed by both Tauri's build script and runtime invoke handler.
fn render_commands(catalog: &DesktopCatalog<'_>) -> String {
    let mut source = format!("{GENERATED_FILE_HEADER}desktop_command_registry! {{\n");
    for (handler, _) in catalog.commands.values() {
        source.push_str(&format!("    {handler},\n"));
    }
    source.push_str("}\n");
    source
}

/// Derives the private command map and stream predicate from the validated binding set.
fn render_transport(catalog: &DesktopCatalog<'_>) -> String {
    let mut source = format!(
        "{GENERATED_FILE_HEADER}import type {{ EndpointOperation }} from \"@ora/contracts\";\n\nexport const tauriStreamOperations = {{\n"
    );
    for operation in catalog.streams.keys() {
        source.push_str(&format!("  {operation}: true,\n"));
    }
    source.push_str("} as const;\n\nexport type TauriStreamOperation = keyof typeof tauriStreamOperations;\n\nexport const tauriCommands = {\n");
    for (operation, command) in &catalog.unary {
        source.push_str(&format!("  {operation}: \"{command}\",\n"));
    }
    source.push_str("} as const satisfies Record<Exclude<EndpointOperation, TauriStreamOperation>, string>;\n\n/** Narrows only operations explicitly bound to Desktop streams. */\nexport function isTauriStreamOperation(operation: string): operation is TauriStreamOperation {\n  return Object.hasOwn(tauriStreamOperations, operation);\n}\n");
    source
}

/// Emits separate allowlists; capability files continue to choose their respective Webviews.
fn render_permission(catalog: &DesktopCatalog<'_>, permission: Permission) -> String {
    let (identifier, description) = match permission {
        Permission::MainWebview => (
            "allow-main-commands",
            "Allows the trusted main Webview to invoke Ora application commands.",
        ),
        Permission::MainAndPluginWebviews => (
            "allow-plugin-webview-invoke",
            "Allows workbench plugin Webviews to invoke the plugin bridge command.",
        ),
    };
    let header = GENERATED_FILE_HEADER.replacen("//", "#", /*count*/ 1);
    let mut source = format!(
        "{header}[[permission]]\nidentifier = \"{identifier}\"\ndescription = \"{description}\"\ncommands.allow = [\n"
    );
    for (command, (_, grant)) in &catalog.commands {
        if permission == Permission::MainWebview || *grant == permission {
            source.push_str(&format!("    \"{command}\",\n"));
        }
    }
    source.push_str("]\n");
    source
}

/// Generates typed stream decoding and dispatch while domains own startup implementation.
fn render_stream_routes(catalog: &DesktopCatalog<'_>) -> String {
    let mut source = format!(
        "{GENERATED_FILE_HEADER}\nuse super::stream::StreamStart;\nuse crate::{{error::CommandError, state::DesktopState}};\nuse tauri::State;\n\n/// Typed requests preserve the public DTO while rejecting unknown operations.\n#[derive(serde::Deserialize)]\n#[serde(tag = \"operationName\", content = \"request\")]\npub(super) enum StreamOperation {{\n"
    );
    for (operation, (endpoint, _)) in &catalog.streams {
        let variant = uppercase_initial(operation);
        source.push_str(&format!(
            "    #[serde(rename = \"{operation}\")]\n    {variant}(ora_contracts::{}),\n",
            endpoint.request_type
        ));
    }
    source.push_str("}\n\n/// Dispatches a request without embedding domain logic in the transport lifecycle.\npub(super) async fn start(\n    state: State<'_, DesktopState>,\n    operation: StreamOperation,\n    context: StreamStart,\n) -> Result<(), CommandError> {\n    match operation {\n");
    for (operation, (_, handler)) in &catalog.streams {
        let variant = uppercase_initial(operation);
        source.push_str(&format!("        StreamOperation::{variant}(request) => {{\n            crate::{handler}(state, request, context).await\n        }}\n"));
    }
    source.push_str("    }\n}\n");
    source
}

/// Preserves a camel-case operation's spelling while giving its Rust variant an uppercase initial.
fn uppercase_initial(operation: &str) -> String {
    let mut chars = operation.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::{render_permission, render_stream_routes, validate};
    use crate::desktop_bindings::{Binding, Permission, bindings};
    use crate::frontend::{FrontendEndpoint, FrontendResponseMode, frontend_endpoints};
    use pretty_assertions::assert_eq;

    const READ: FrontendEndpoint = FrontendEndpoint {
        operation_name: "readFixture",
        namespace: "fixture",
        member_name: "read",
        request_type: "FixtureRequest",
        response_type: "FixtureResponse",
        response_mode: FrontendResponseMode::Unary,
    };
    const READ_BINDING: Binding = Binding::Unary {
        operation: "readFixture",
        handler: "commands::fixture::read",
        permission: Permission::MainWebview,
    };

    /// Invalid joins fail before generation instead of relying on a runtime unsupported route.
    #[test]
    fn rejects_missing_unknown_duplicate_and_wrong_mode_bindings() {
        let cases = [
            (Vec::new(), "missing Desktop bindings: readFixture"),
            (
                vec![READ_BINDING, READ_BINDING],
                "duplicate Desktop operation binding: readFixture",
            ),
            (
                vec![Binding::Stream {
                    operation: "readFixture",
                    handler: "commands::fixture::read",
                }],
                "Desktop binding mode disagrees with operation: readFixture",
            ),
            (
                vec![Binding::Unary {
                    operation: "unknown",
                    handler: "commands::fixture::read",
                    permission: Permission::MainWebview,
                }],
                "Desktop binding refers to unknown operation: unknown",
            ),
            (
                vec![
                    READ_BINDING,
                    Binding::Native {
                        handler: "commands::other::read",
                        permission: Permission::MainWebview,
                    },
                ],
                "duplicate Desktop command: read",
            ),
        ];
        for (bindings, expected) in cases {
            assert_eq!(
                validate(&[READ], &bindings).err(),
                Some(expected.to_string())
            );
        }
    }

    /// An unknown stream name obtains a typed route from declarations without another name table.
    #[test]
    fn new_streams_require_the_shared_commands_and_generate_typed_dispatch() {
        let endpoint = FrontendEndpoint {
            response_mode: FrontendResponseMode::Stream,
            ..READ
        };
        let mut bindings = vec![Binding::Stream {
            operation: "readFixture",
            handler: "commands::fixture::start",
        }];
        assert_eq!(
            validate(&[endpoint], &bindings).err(),
            Some("missing shared stream command: stream_contract".to_string())
        );
        for handler in [
            "commands::stream::stream_contract",
            "commands::stream::cancel_contract_stream",
        ] {
            bindings.push(Binding::Native {
                handler,
                permission: Permission::MainWebview,
            });
        }
        let endpoints = [endpoint];
        let catalog = validate(&endpoints, &bindings)
            .unwrap_or_else(|error| panic!("validate fixture: {error}"));
        let routes = render_stream_routes(&catalog);
        assert!(routes.contains("ReadFixture(ora_contracts::FixtureRequest)"));
        assert!(routes.contains("crate::commands::fixture::start(state, request, context).await"));
        bindings.remove(0);
        let empty = validate(&[], &bindings)
            .unwrap_or_else(|error| panic!("validate removed fixture: {error}"));
        assert!(!render_stream_routes(&empty).contains("ReadFixture"));
    }

    /// Isolated plugin Webviews retain only the existing bridge even as application commands grow.
    #[test]
    fn public_catalog_is_complete_and_plugin_grants_remain_restricted() {
        let endpoints = frontend_endpoints();
        let catalog = validate(&endpoints, &bindings())
            .unwrap_or_else(|error| panic!("validate Desktop catalog: {error}"));
        let permission = render_permission(&catalog, Permission::MainAndPluginWebviews);
        assert_eq!(
            permission
                .lines()
                .filter(|line| line.starts_with("    \""))
                .collect::<Vec<_>>(),
            vec!["    \"plugin_webview_invoke\","]
        );
        assert_eq!(catalog.unary.len() + catalog.streams.len(), endpoints.len());
    }
}
