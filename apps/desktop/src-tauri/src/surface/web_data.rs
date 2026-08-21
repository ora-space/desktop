//! Resolves a manifest `WebDataPolicy` into the platform mechanism that isolates web data.

use ora_logging::ora_warn;
use ora_plugin_manager::WebDataPolicy;
use ora_surface::SurfaceDefinitionId;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Directory below the plugin data directory that holds per-surface web profiles.
const WEB_PROFILE_DIRECTORY: &str = "web-profile";

/// How the webview must be configured so the surface gets the requested data isolation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedWebData {
    /// Windows / Linux: a dedicated profile directory (`WebView2` user data folder, WebKitGTK
    /// website data directory).
    Directory(PathBuf),
    /// macOS: a `WKWebsiteDataStore` identifier; the OS owns the storage location.
    StoreIdentifier([u8; 16]),
    /// Nothing is persisted and nothing is shared.
    Incognito,
    /// Persistence was requested but this platform cannot isolate it; the surface shares the
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

/// Picks the isolation mechanism for one surface and prepares it (creates the profile directory).
///
/// Persistent profiles are keyed by `(plugin, surface)` so two surfaces of one plugin never
/// share cookies, and so the profile survives instance ids, which are process-local.
pub fn resolve(
    policy: WebDataPolicy,
    definition: &SurfaceDefinitionId,
    plugin_data_directory: &Path,
    platform: HostPlatform,
) -> io::Result<ResolvedWebData> {
    match (policy, platform) {
        (WebDataPolicy::EphemeralIsolated, _) => Ok(ResolvedWebData::Incognito),
        (WebDataPolicy::PersistentProfile, HostPlatform::Windows | HostPlatform::Linux) => {
            let directory = plugin_data_directory
                .join(WEB_PROFILE_DIRECTORY)
                .join(definition.surface_id.as_str());
            std::fs::create_dir_all(&directory)?;
            Ok(ResolvedWebData::Directory(directory))
        }
        (WebDataPolicy::PersistentProfile, HostPlatform::MacOs) => Ok(
            ResolvedWebData::StoreIdentifier(store_identifier(definition)),
        ),
        (WebDataPolicy::PersistentProfile, HostPlatform::Other) => {
            ora_warn!(
                message = "persistent web profile is not isolated on this platform; sharing the default store",
                plugin_id = %definition.plugin_id,
                surface_id = definition.surface_id.as_str(),
            );
            Ok(ResolvedWebData::SharedDefault)
        }
    }
}

/// Derives the macOS data store identifier as UUID v5 of `{plugin_id}/{surface_id}`.
///
/// The URL namespace is used with an `ora://surface/` prefix so the identifiers can never
/// collide with a v5 id another component derives from a plain plugin id.
fn store_identifier(definition: &SurfaceDefinitionId) -> [u8; 16] {
    let name = format!(
        "ora://surface/{}/{}",
        definition.plugin_id,
        definition.surface_id.as_str()
    );
    *Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::{HostPlatform, ResolvedWebData, resolve};
    use ora_domain::PluginId;
    use ora_plugin_manager::{SurfaceId, WebDataPolicy};
    use ora_surface::SurfaceDefinitionId;
    use pretty_assertions::assert_eq;

    /// Builds the SkillHub market surface id used by every case.
    fn definition() -> SurfaceDefinitionId {
        SurfaceDefinitionId {
            plugin_id: PluginId::new("official", "ora-space.skillhub").expect("plugin id"),
            surface_id: SurfaceId::parse("market").expect("valid surface id"),
        }
    }

    /// Verifies every policy/platform pair maps to the documented mechanism and that the
    /// Linux/Windows profile directory is created.
    #[test]
    fn resolves_policy_per_platform() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data = temp.path();
        let definition = definition();
        let expected_directory = data.join("web-profile").join("market");
        let resolved = [
            (WebDataPolicy::PersistentProfile, HostPlatform::Linux),
            (WebDataPolicy::PersistentProfile, HostPlatform::Windows),
            (WebDataPolicy::PersistentProfile, HostPlatform::MacOs),
            (WebDataPolicy::PersistentProfile, HostPlatform::Other),
            (WebDataPolicy::EphemeralIsolated, HostPlatform::Linux),
        ]
        .map(|(policy, platform)| resolve(policy, &definition, data, platform).expect("resolve"));
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
                    ResolvedWebData::Incognito,
                ],
                true
            )
        );
    }

    /// Verifies the macOS identifier is stable and differs between surfaces of one plugin.
    #[test]
    fn store_identifier_is_stable_per_surface() {
        let temp = tempfile::tempdir().expect("tempdir");
        let market = definition();
        let docs = SurfaceDefinitionId {
            plugin_id: PluginId::new("official", "ora-space.skillhub").expect("plugin id"),
            surface_id: SurfaceId::parse("docs").expect("valid surface id"),
        };
        let resolve_macos = |definition: &SurfaceDefinitionId| {
            resolve(
                WebDataPolicy::PersistentProfile,
                definition,
                temp.path(),
                HostPlatform::MacOs,
            )
            .expect("resolve")
        };
        assert_eq!(
            (
                resolve_macos(&market) == resolve_macos(&market),
                resolve_macos(&market) == resolve_macos(&docs),
            ),
            (true, false)
        );
    }
}
