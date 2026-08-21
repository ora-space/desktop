use crate::ids::SurfaceDefinitionId;
use crate::navigation::NavigationPolicy;
use ora_domain::PluginId;
use ora_plugin_manager::{
    InstalledSurface, InstalledSurfaceSource, InstancePolicy, PanelSource, RemoteSiteSource,
    WebDataPolicy,
};
use ora_utils::path::PortableRelativePath;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use url::Url;

/// One surface as the host understands it: identity, title, content source, instance policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceDefinition {
    pub id: SurfaceDefinitionId,
    pub title: String,
    pub source: SurfaceSource,
    pub instance_policy: InstancePolicy,
}

/// Where the surface content comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceSource {
    RemoteSite(RemoteSiteDefinition),
    Panel(PanelDefinition),
}

/// A remote web site shown inside an isolated webview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSiteDefinition {
    pub entry_url: Url,
    pub navigation: NavigationPolicy,
    pub web_data: WebDataPolicy,
}

/// A page shipped inside the plugin package and served by the host from `asset_root`.
///
/// Panels carry no web data policy: the host always gives them a web profile of their own (one
/// per plugin and surface) so panels of different plugins never see each other's storage on
/// platforms where they share one origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelDefinition {
    /// Canonical directory below which every servable file lives.
    pub asset_root: PathBuf,
    /// Entry document relative to `asset_root`.
    pub entry: PortableRelativePath,
}

/// Where an instance is mounted: inside the host window or as its own window.
///
/// Serialized lowercase because the frontend event contract spells targets as
/// `"embedded"` / `"windowed"`; deserialized with the same spelling for `surface_open`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MountTarget {
    Embedded,
    Windowed,
}

impl SurfaceDefinition {
    /// Builds a definition from a manifest surface that `ora-plugin-manager` already validated.
    ///
    /// No invariant is re-checked here on purpose: the installed types are the single source of
    /// truth and this is a pure type transfer.
    pub fn from_installed(plugin_id: &PluginId, surface: &InstalledSurface) -> Self {
        let source = match &surface.source {
            InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
                entry_url,
                allow_hosts,
                allow_host_suffixes,
                web_data,
            }) => SurfaceSource::RemoteSite(RemoteSiteDefinition {
                entry_url: entry_url.clone(),
                navigation: NavigationPolicy::remote_site(
                    allow_hosts.clone(),
                    allow_host_suffixes.clone(),
                ),
                web_data: *web_data,
            }),
            InstalledSurfaceSource::Panel(PanelSource { asset_root, entry }) => {
                SurfaceSource::Panel(PanelDefinition {
                    asset_root: asset_root.clone(),
                    entry: entry.clone(),
                })
            }
        };
        Self {
            id: SurfaceDefinitionId {
                plugin_id: plugin_id.clone(),
                surface_id: surface.id.clone(),
            },
            title: surface.title.clone(),
            source,
            instance_policy: surface.instance_policy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MountTarget, PanelDefinition, RemoteSiteDefinition, SurfaceDefinition, SurfaceSource,
    };
    use crate::ids::SurfaceDefinitionId;
    use crate::navigation::NavigationPolicy;
    use ora_domain::PluginId;
    use ora_plugin_manager::{
        HostName, InstalledSurface, InstalledSurfaceSource, InstancePolicy, PanelSource,
        RemoteSiteSource, SurfaceId, WebDataPolicy,
    };
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;
    use url::Url;

    /// Verifies the installed manifest shape is carried over field by field.
    #[test]
    fn builds_definition_from_installed_surface() {
        let plugin_id = PluginId::new("ora-space.skillhub");
        let entry_url = Url::parse("https://www.skillhub.cn").expect("valid url");
        let exact = vec![HostName::parse("www.skillhub.cn").expect("valid host")];
        let suffixes = vec![HostName::parse("skillhub.cn").expect("valid host")];
        let installed = InstalledSurface {
            id: SurfaceId::parse("market").expect("valid surface id"),
            title: "SkillHub".to_owned(),
            instance_policy: InstancePolicy::Singleton,
            source: InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
                entry_url: entry_url.clone(),
                allow_hosts: exact.clone(),
                allow_host_suffixes: suffixes.clone(),
                web_data: WebDataPolicy::PersistentProfile,
            }),
        };

        assert_eq!(
            SurfaceDefinition::from_installed(&plugin_id, &installed),
            SurfaceDefinition {
                id: SurfaceDefinitionId {
                    plugin_id,
                    surface_id: SurfaceId::parse("market").expect("valid surface id"),
                },
                title: "SkillHub".to_owned(),
                source: SurfaceSource::RemoteSite(RemoteSiteDefinition {
                    entry_url,
                    navigation: NavigationPolicy::remote_site(exact, suffixes),
                    web_data: WebDataPolicy::PersistentProfile,
                }),
                instance_policy: InstancePolicy::Singleton,
            }
        );
    }

    /// Verifies a panel carries its asset root and entry through unchanged.
    #[test]
    fn builds_definition_from_installed_panel() {
        let plugin_id = PluginId::new("ora-space.hello-panel");
        let installed = InstalledSurface {
            id: SurfaceId::parse("counter").expect("valid surface id"),
            title: "Hello Panel".to_owned(),
            instance_policy: InstancePolicy::Singleton,
            source: InstalledSurfaceSource::Panel(PanelSource {
                asset_root: PathBuf::from("/plugins/hello-panel/ui"),
                entry: PortableRelativePath::parse("index.html").expect("entry"),
            }),
        };

        assert_eq!(
            SurfaceDefinition::from_installed(&plugin_id, &installed),
            SurfaceDefinition {
                id: SurfaceDefinitionId {
                    plugin_id,
                    surface_id: SurfaceId::parse("counter").expect("valid surface id"),
                },
                title: "Hello Panel".to_owned(),
                source: SurfaceSource::Panel(PanelDefinition {
                    asset_root: PathBuf::from("/plugins/hello-panel/ui"),
                    entry: PortableRelativePath::parse("index.html").expect("entry"),
                }),
                instance_policy: InstancePolicy::Singleton,
            }
        );
    }

    /// Verifies the wire spelling the frontend adapter relies on.
    #[test]
    fn mount_target_serializes_lowercase() {
        assert_eq!(
            [MountTarget::Embedded, MountTarget::Windowed]
                .map(|target| serde_json::to_string(&target).expect("serialize")),
            ["\"embedded\"".to_owned(), "\"windowed\"".to_owned()]
        );
    }
}
