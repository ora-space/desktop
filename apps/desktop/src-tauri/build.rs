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

fn main() {
    println!("cargo:rerun-if-changed=src/app_commands.rs");
    let commands = desktop_commands();
    assert!(
        !commands.is_empty(),
        "src/app_commands.rs declares no commands; the ACL manifest would allow nothing"
    );
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
