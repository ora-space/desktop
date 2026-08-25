use std::{fs, path::Path};

macro_rules! desktop_command_registry {
    ($($module:ident::$command:ident),* $(,)?) => {
        &[$(stringify!($command)),*]
    };
}

const DESKTOP_COMMANDS: &[&str] = include!("src/app_commands.rs");

/// Ensures every registered desktop command can be invoked by the trusted main Webview.
fn validate_main_command_permissions() {
    let permission_path = Path::new("permissions").join("main-commands.toml");
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
    let missing_commands = DESKTOP_COMMANDS
        .iter()
        .filter(|command| !allowed_commands.contains(**command))
        .copied()
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
    validate_main_command_permissions();

    // Drop Tauri's resource-embedded app manifest and attach Common-Controls v6 via
    // the linker instead. Resource manifests only land on bins; cargo's lib-test
    // harness is not a bin, so it otherwise binds legacy comctl32 and dies at load
    // with STATUS_ENTRYPOINT_NOT_FOUND (tauri#13419 / TaskDialogIndirect).
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
        .app_manifest(tauri_build::AppManifest::new().commands(DESKTOP_COMMANDS));
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
