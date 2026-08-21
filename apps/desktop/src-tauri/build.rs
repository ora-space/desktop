macro_rules! desktop_command_registry {
    ($($module:ident::$command:ident),* $(,)?) => {
        &[$(stringify!($command)),*]
    };
}

const DESKTOP_COMMANDS: &[&str] = include!("src/app_commands.rs");

fn main() {
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
