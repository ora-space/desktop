//! Decides whether the running installation may replace itself with a downloaded package.
//!
//! `tauri_plugin_updater` dispatches its installer on the bundle type compiled into the binary and
//! resolves the Linux replacement target from the `APPIMAGE` environment variable. The static
//! manifest only advertises an AppImage for Linux, so a `deb` or `rpm` installation would hand its
//! package manager an artifact it cannot read. Detecting that here keeps the failure out of the
//! install path and lets the webview explain the real remedy instead.

use super::ManualUpdateReason;

/// Reports whether the Tauri updater can install into this particular installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InstallSupport {
    /// The updater can replace the running package in place.
    SelfUpdating,
    /// The user has to update through the channel named by the reason.
    ///
    /// Linux is the only target that can currently produce this: Windows ships a single bundle
    /// format that the updater always owns.
    #[cfg_attr(target_os = "windows", expect(dead_code))]
    Manual(ManualUpdateReason),
}

/// Classifies the running installation for the current target platform.
#[cfg(target_os = "linux")]
pub(super) fn install_support() -> InstallSupport {
    use tauri::utils::config::BundleType;

    match tauri::utils::platform::bundle_type() {
        Some(BundleType::Deb | BundleType::Rpm) => {
            InstallSupport::Manual(ManualUpdateReason::SystemPackage)
        }
        // Without `APPIMAGE` the updater would resolve the replacement target to the bare
        // executable and unpack an AppImage over it.
        _ if std::env::var_os("APPIMAGE").is_none() => {
            InstallSupport::Manual(ManualUpdateReason::UnpackagedBinary)
        }
        _ => InstallSupport::SelfUpdating,
    }
}

/// Classifies a Windows installation, which the NSIS updater always owns.
#[cfg(target_os = "windows")]
pub(super) fn install_support() -> InstallSupport {
    InstallSupport::SelfUpdating
}
