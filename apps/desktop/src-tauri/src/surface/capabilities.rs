//! Compile-time and runtime detection of embedded (child webview) support.

use ora_logging::ora_info;
use serde::Serialize;

/// What the frontend may ask of this build, reported once by `surface_capabilities`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceCapabilities {
    pub embedded: bool,
    pub web_data_isolation: WebDataIsolation,
}

/// Whether persistent web profiles are really isolated per surface on this platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WebDataIsolation {
    Isolated,
    Degraded,
}

impl SurfaceCapabilities {
    /// Computes the capabilities from the build features and the process environment.
    ///
    /// Embedded surfaces require the `embedded-surfaces` feature and a windowing backend wry can
    /// child-parent into: on Linux that is X11 only, so a Wayland session without a forced X11
    /// GDK backend falls back to windowed surfaces.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> Self {
        let compiled = cfg!(feature = "embedded-surfaces");
        let platform_supported = if cfg!(target_os = "linux") {
            let wayland = env("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
            let forced_x11 = env("GDK_BACKEND").is_some_and(|value| value == "x11");
            !wayland || forced_x11
        } else {
            true
        };
        let capabilities = Self {
            embedded: compiled && platform_supported,
            web_data_isolation: if cfg!(any(
                target_os = "linux",
                target_os = "windows",
                target_os = "macos"
            )) {
                WebDataIsolation::Isolated
            } else {
                WebDataIsolation::Degraded
            },
        };
        ora_info!(
            message = "surface capabilities detected",
            embedded = capabilities.embedded,
            embedded_compiled = compiled,
            embedded_platform_supported = platform_supported,
            web_data_isolation = ?capabilities.web_data_isolation,
        );
        capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::SurfaceCapabilities;
    #[cfg(target_os = "linux")]
    use super::WebDataIsolation;
    use pretty_assertions::assert_eq;

    /// Verifies a Wayland session without the X11 override never reports embedded support.
    #[cfg(target_os = "linux")]
    #[test]
    fn wayland_without_x11_override_disables_embedded() {
        let capabilities = SurfaceCapabilities::detect(|key| match key {
            "WAYLAND_DISPLAY" => Some("wayland-0".to_owned()),
            _ => None,
        });
        assert_eq!(
            capabilities,
            SurfaceCapabilities {
                embedded: false,
                web_data_isolation: WebDataIsolation::Isolated,
            }
        );
    }

    /// Verifies the default build reports no embedded support even on a supported display.
    #[test]
    fn default_build_reports_embedded_only_with_feature() {
        let capabilities = SurfaceCapabilities::detect(|key| match key {
            "GDK_BACKEND" => Some("x11".to_owned()),
            _ => None,
        });
        assert_eq!(capabilities.embedded, cfg!(feature = "embedded-surfaces"));
    }
}
