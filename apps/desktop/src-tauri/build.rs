// Shared with `src/lib.rs` so the ACL manifest and the IPC handler list cannot drift.
include!("app_commands.rs");

fn main() {
    // The app ACL manifest is rebuilt from the command list and the permission set
    // files; cargo only knows about them if we declare the dependency explicitly.
    println!("cargo:rerun-if-changed=app_commands.rs");
    println!("cargo:rerun-if-changed=permissions");
    println!("cargo:rerun-if-changed=capabilities");

    // Drop Tauri's resource-embedded app manifest and attach Common-Controls v6 via
    // the linker instead. Resource manifests only land on bins; cargo's lib-test
    // harness is not a bin, so it otherwise binds legacy comctl32 and dies at load
    // with STATUS_ENTRYPOINT_NOT_FOUND (tauri#13419 / TaskDialogIndirect).
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
        // Declaring the commands makes Tauri enforce the ACL on every app command, so
        // only webviews covered by a capability (the `main` webview) can invoke them.
        .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS));
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
