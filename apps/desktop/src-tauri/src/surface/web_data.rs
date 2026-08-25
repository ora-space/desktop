//! Resolves the platform mechanism that gives one plugin an isolated, persistent browser profile.
//!
//! Both surface kinds get a persistent profile keyed by plugin id: a webview plugin keeps its
//! login state across restarts, and a workbench plugin keeps its own `localStorage` separate from
//! every other plugin on platforms where all `ora-plugin://` pages share one origin. There is no
//! ephemeral mode in v1.

use ora_domain::PluginId;
use ora_logging::ora_warn;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Directory below the plugin data directory that holds the browser profile.
const WEB_PROFILE_DIRECTORY: &str = "web-profile";

/// How the webview must be configured so the plugin gets an isolated persistent profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedWebData {
    /// Windows / Linux: a dedicated profile directory (WebView2 user data folder, WebKitGTK
    /// website data directory).
    Directory(PathBuf),
    /// macOS: a `WKWebsiteDataStore` identifier; the OS owns the storage location.
    StoreIdentifier([u8; 16]),
    /// Persistence was requested but this platform cannot isolate it; the plugin shares the
    /// system default store with every other surface.
    SharedDefault,
}

/// The platforms whose web data mechanisms differ; resolved at compile time in production.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPlatform {
    Windows,
    Linux,
    MacOs,
    Other,
}

impl HostPlatform {
    /// The platform this binary was compiled for.
    pub const CURRENT: Self = if cfg!(target_os = "windows") {
        Self::Windows
    } else if cfg!(target_os = "linux") {
        Self::Linux
    } else if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Other
    };
}

/// Picks the isolation mechanism for one plugin and prepares it (creates the profile directory).
///
/// The profile is keyed by plugin id, not by instance, so it survives process-local instance
/// ids and a version upgrade of the same plugin.
pub fn resolve(
    plugin_id: &PluginId,
    plugin_data_directory: &Path,
    platform: HostPlatform,
) -> io::Result<ResolvedWebData> {
    match platform {
        HostPlatform::Windows | HostPlatform::Linux => {
            let directory = plugin_data_directory.join(WEB_PROFILE_DIRECTORY);
            std::fs::create_dir_all(&directory)?;
            Ok(ResolvedWebData::Directory(directory))
        }
        HostPlatform::MacOs => Ok(ResolvedWebData::StoreIdentifier(store_identifier(
            plugin_id,
        ))),
        HostPlatform::Other => {
            ora_warn!(
                message = "persistent web profile is not isolated on this platform; sharing the default store",
                plugin_id = %plugin_id,
            );
            Ok(ResolvedWebData::SharedDefault)
        }
    }
}

/// Derives the macOS data store identifier as UUID v5 of `ora://plugin/{plugin_id}`.
///
/// The URL namespace is used with an `ora://plugin/` prefix so the identifiers can never collide
/// with a v5 id another component derives from a plain plugin id.
fn store_identifier(plugin_id: &PluginId) -> [u8; 16] {
    let name = format!("ora://plugin/{plugin_id}");
    *Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::{HostPlatform, ResolvedWebData, resolve};
    use ora_domain::PluginId;
    use pretty_assertions::assert_eq;

    fn plugin() -> PluginId {
        PluginId::new("official", "acme.hub").expect("plugin id")
    }

    /// Every platform maps to the documented mechanism, and the profile directory is created.
    #[test]
    fn resolves_per_platform() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data = temp.path();
        let plugin = plugin();
        let expected_directory = data.join("web-profile");
        let resolved = [
            HostPlatform::Linux,
            HostPlatform::Windows,
            HostPlatform::MacOs,
            HostPlatform::Other,
        ]
        .map(|platform| resolve(&plugin, data, platform).expect("resolve"));
        let macos_store = match &resolved[2] {
            ResolvedWebData::StoreIdentifier(bytes) => *bytes,
            other => panic!("expected store identifier, got {other:?}"),
        };
        assert_eq!(
            (resolved, expected_directory.is_dir()),
            (
                [
                    ResolvedWebData::Directory(expected_directory.clone()),
                    ResolvedWebData::Directory(expected_directory.clone()),
                    ResolvedWebData::StoreIdentifier(macos_store),
                    ResolvedWebData::SharedDefault,
                ],
                true
            )
        );
    }

    /// The macOS identifier is stable per plugin and differs between plugins.
    #[test]
    fn store_identifier_is_stable_per_plugin() {
        let temp = tempfile::tempdir().expect("tempdir");
        let other = PluginId::new("official", "acme.tools").expect("plugin id");
        let resolve_macos =
            |plugin: &PluginId| resolve(plugin, temp.path(), HostPlatform::MacOs).expect("resolve");
        assert_eq!(
            (
                resolve_macos(&plugin()) == resolve_macos(&plugin()),
                resolve_macos(&plugin()) == resolve_macos(&other),
            ),
            (true, false)
        );
    }
}
