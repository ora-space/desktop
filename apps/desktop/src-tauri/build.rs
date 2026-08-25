use std::{fs, path::Path};

/// The single command registry shared with `lib.rs`, where it expands to the invoke handler.
const DESKTOP_COMMAND_REGISTRY: &str = include_str!("src/app_commands.rs");

/// Extracts every command name from the registry so the Tauri app manifest enumerates exactly the
/// commands the runtime handler registers.
///
/// The registry is read as text rather than through a macro because `generate_handler!` accepts
/// arbitrary module paths (`surface::commands::surface_open`), which `macro_rules!` cannot split
/// into "path" and "last segment" without ambiguity. One entry per line, ending in a comma, is
/// the only shape the file takes.
fn desktop_commands() -> Vec<String> {
    DESKTOP_COMMAND_REGISTRY
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .filter_map(|line| line.strip_suffix(','))
        .map(|path| path.rsplit("::").next().unwrap_or(path).to_string())
        .collect()
}

/// Ensures every registered desktop command can be invoked by the trusted main Webview.
///
/// Parses `commands.allow` from TOML rather than scanning quoted lines so permission metadata
/// keys cannot be mistaken for grants. The path is rooted at `CARGO_MANIFEST_DIR` so the check
/// stays correct when cargo invokes the build script from another working directory.
fn validate_main_command_permissions(commands: &[String]) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set when cargo runs a build script");
    let permission_path = Path::new(&manifest_dir)
        .join("permissions")
        .join("main-commands.toml");
    let permission_source = fs::read_to_string(&permission_path).unwrap_or_else(|error| {
        panic!(
            "failed to read desktop command permissions from {}: {error}",
            permission_path.display()
        )
    });
    let permissions = toml::from_str::<toml::Value>(&permission_source)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", permission_path.display()));
    let allowed_commands = permissions
        .get("permission")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|permission| permission.get("commands"))
        .filter_map(toml::Value::as_table)
        .filter_map(|commands| commands.get("allow"))
        .filter_map(toml::Value::as_array)
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let missing_commands = commands
        .iter()
        .filter(|command| !allowed_commands.contains(command.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    // Tauri registers commands independently from its capability allowlist, so a missing
    // permission otherwise survives compilation and only fails when a user opens the feature.
    assert!(
        missing_commands.is_empty(),
        "registered desktop commands missing from {}: {}",
        permission_path.display(),
        missing_commands.join(", ")
    );
}

fn main() {
    println!("cargo:rerun-if-changed=permissions/main-commands.toml");
    println!("cargo:rerun-if-changed=src/app_commands.rs");
    let commands = desktop_commands();
    assert!(
        !commands.is_empty(),
        "src/app_commands.rs declares no commands; the ACL manifest would allow nothing"
    );
    // The ACL invariant (README "Command ACL"): every command in the registry must be granted to
    // the main Webview in `permissions/main-commands.toml`. A command registered here but missing
    // from that file is callable by nobody, so the main Webview invoke is denied at runtime and
    // surfaces to the user as "tauri_invoke_failure". Enforce it at build time so the two files
    // cannot drift.
    validate_main_command_permissions(&commands);
    // `AppManifest::commands` keeps a `'static` slice; leaking the build-time list is the
    // cheapest way to hand it one, and a build script's memory ends with the process anyway.
    let command_names: &'static [&'static str] = Box::leak(
        commands
            .into_iter()
            .map(|command| &*Box::leak(command.into_boxed_str()))
            .collect::<Vec<&'static str>>()
            .into_boxed_slice(),
    );
    // Drop Tauri's resource-embedded app manifest and attach Common-Controls v6 via
    // the linker instead. Resource manifests only land on bins; cargo's lib-test
    // harness is not a bin, so it otherwise binds legacy comctl32 and dies at load
    // with STATUS_ENTRYPOINT_NOT_FOUND (tauri#13419 / TaskDialogIndirect).
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
        .app_manifest(tauri_build::AppManifest::new().commands(command_names));
    tauri_build::try_build(attributes).expect("failed to run tauri-build");

    #[cfg(windows)]
    {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
