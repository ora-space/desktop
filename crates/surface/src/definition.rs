use crate::navigation::NavigationPolicy;
use ora_domain::PluginId;
use ora_plugin_manager::{InstalledPlugin, PluginContribution};
use ora_plugin_manifest::{DownloadPolicy, MethodName};
use ora_utils::path::PortableRelativePath;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use url::Url;

/// One surface as the host understands it: the plugin, its title, and where content comes from.
///
/// A definition is an immutable snapshot taken from the installed package when a surface opens;
/// an instance keeps its own copy so a plugin upgrade never changes what an already open page
/// may load, call, navigate to, or do with downloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceDefinition {
    pub plugin_id: PluginId,
    pub title: String,
    pub source: SurfaceSource,
}

/// Where the surface content comes from; also decides the label family and instance policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceSource {
    Workbench(WorkbenchDefinition),
    RemoteSite(RemoteSiteDefinition),
}

/// A page shipped inside the plugin package and served by the host from `asset_root`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkbenchDefinition {
    /// Canonical directory below which every servable file lives.
    pub asset_root: PathBuf,
    /// Entry document relative to `asset_root`.
    pub page_entry: PortableRelativePath,
    /// Methods the manifest exposes to the page; intersected with the running registration.
    pub declared_methods: Vec<MethodName>,
}

/// An external HTTPS site shown inside an isolated webview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSiteDefinition {
    pub start_url: Url,
    pub navigation: NavigationPolicy,
    pub download_policy: DownloadPolicy,
}

/// How many live instances one definition may have.
///
/// A remote site is a singleton per plugin because every instance would share the plugin's
/// browser profile; a workbench page keeps its own state per instance and may open many times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstancePolicy {
    Singleton,
    Multiple,
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

/// The two surface kinds as spelled on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceKind {
    Workbench,
    Webview,
}

impl SurfaceDefinition {
    /// Builds a definition from an installed plugin that `ora-plugin-manager` already validated,
    /// or `None` when the plugin contributes no surface (an agent).
    ///
    /// No invariant is re-checked here on purpose: the installed types are the single source of
    /// truth and this is a pure type transfer.
    pub fn from_installed(plugin: &InstalledPlugin) -> Option<Self> {
        let source = match &plugin.contributes {
            PluginContribution::Agent(_) | PluginContribution::Skill(_) => return None,
            PluginContribution::Workbench(descriptor) => {
                SurfaceSource::Workbench(WorkbenchDefinition {
                    asset_root: descriptor.asset_root.clone(),
                    page_entry: descriptor.page_entry.clone(),
                    declared_methods: descriptor.declared_methods.clone(),
                })
            }
            PluginContribution::Webview(descriptor) => {
                SurfaceSource::RemoteSite(RemoteSiteDefinition {
                    start_url: descriptor.start_url.as_url().clone(),
                    navigation: NavigationPolicy::remote_site(descriptor.allowed_origins.clone()),
                    download_policy: descriptor.download_policy.clone(),
                })
            }
        };
        Some(Self {
            plugin_id: plugin.id.clone(),
            title: plugin.display_name.clone(),
            source,
        })
    }

    /// Returns the wire spelling of this definition's kind.
    pub fn kind(&self) -> SurfaceKind {
        match &self.source {
            SurfaceSource::Workbench(_) => SurfaceKind::Workbench,
            SurfaceSource::RemoteSite(_) => SurfaceKind::Webview,
        }
    }

    /// Returns how many live instances this definition may have.
    pub fn instance_policy(&self) -> InstancePolicy {
        match &self.source {
            SurfaceSource::Workbench(_) => InstancePolicy::Multiple,
            SurfaceSource::RemoteSite(_) => InstancePolicy::Singleton,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InstancePolicy, MountTarget, RemoteSiteDefinition, SurfaceDefinition, SurfaceKind,
        SurfaceSource, WorkbenchDefinition,
    };
    use crate::navigation::NavigationPolicy;
    use ora_domain::PluginId;
    use ora_plugin_manager::{
        InstalledPlugin, InstalledPluginAgent, InstalledWebviewDescriptor,
        InstalledWorkbenchDescriptor, PluginContribution,
    };
    use ora_plugin_manifest::{DownloadPolicy, MethodName, Origin, StartUrl};
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use semver::Version;
    use std::path::PathBuf;
    use url::Url;

    /// Builds one installed plugin around a contribution.
    fn installed(name: &str, contributes: PluginContribution) -> InstalledPlugin {
        InstalledPlugin {
            package_root: PathBuf::from("/plugins/x"),
            id: PluginId::new("official", name).expect("plugin id"),
            version: Version::new(0, 1, 0),
            display_name: name.to_owned(),
            description: String::new(),
            homepage: None,
            license: None,
            contributes,
            logo: None,
            configuration_declaration:
                ora_plugin_manager::PluginConfigurationDeclarationValidity::NotDeclared,
        }
    }

    /// A webview plugin becomes a singleton remote-site definition, a workbench plugin a
    /// multi-instance page definition, and agents and skills none at all.
    #[test]
    fn maps_installed_contributions_to_definitions() {
        let webview = installed(
            "acme.hub",
            PluginContribution::Webview(InstalledWebviewDescriptor {
                start_url: StartUrl::parse("https://www.example.com/skills").expect("url"),
                allowed_origins: vec![Origin::parse("https://www.example.com").expect("origin")],
                download_policy: DownloadPolicy::default(),
            }),
        );
        let workbench = installed(
            "acme.panel",
            PluginContribution::Workbench(InstalledWorkbenchDescriptor {
                entrypoint: PortableRelativePath::parse("main.js").expect("entrypoint"),
                asset_root: PathBuf::from("/plugins/hello/assets"),
                page_entry: PortableRelativePath::parse("index.html").expect("entry"),
                declared_methods: vec![MethodName::parse("counter/get").expect("method")],
            }),
        );
        let agent = installed(
            "acme.agent",
            PluginContribution::Agent(InstalledPluginAgent {
                display_name: "Claude".to_owned(),
                entrypoint: PortableRelativePath::parse("main.js").expect("entrypoint"),
            }),
        );
        let skill = installed("acme.skill", PluginContribution::Skill(Default::default()));

        let webview_definition = SurfaceDefinition::from_installed(&webview);
        let workbench_definition = SurfaceDefinition::from_installed(&workbench);
        assert_eq!(
            (
                webview_definition.clone(),
                webview_definition.as_ref().map(SurfaceDefinition::kind),
                webview_definition
                    .as_ref()
                    .map(SurfaceDefinition::instance_policy),
                workbench_definition.as_ref().map(SurfaceDefinition::kind),
                workbench_definition
                    .as_ref()
                    .map(SurfaceDefinition::instance_policy),
                SurfaceDefinition::from_installed(&agent),
                SurfaceDefinition::from_installed(&skill),
            ),
            (
                Some(SurfaceDefinition {
                    plugin_id: PluginId::new("official", "acme.hub").expect("plugin id"),
                    title: "acme.hub".to_owned(),
                    source: SurfaceSource::RemoteSite(RemoteSiteDefinition {
                        start_url: Url::parse("https://www.example.com/skills").expect("url"),
                        navigation: NavigationPolicy::remote_site(vec![
                            Origin::parse("https://www.example.com").expect("origin"),
                        ]),
                        download_policy: DownloadPolicy::default(),
                    }),
                }),
                Some(SurfaceKind::Webview),
                Some(InstancePolicy::Singleton),
                Some(SurfaceKind::Workbench),
                Some(InstancePolicy::Multiple),
                None,
                None,
            )
        );
        assert_eq!(
            workbench_definition.map(|definition| definition.source),
            Some(SurfaceSource::Workbench(WorkbenchDefinition {
                asset_root: PathBuf::from("/plugins/hello/assets"),
                page_entry: PortableRelativePath::parse("index.html").expect("entry"),
                declared_methods: vec![MethodName::parse("counter/get").expect("method")],
            }))
        );
    }

    /// Mount targets and kinds use the lowercase wire spelling in both directions.
    #[test]
    fn mount_target_and_kind_round_trip_lowercase() {
        assert_eq!(
            (
                serde_json::to_string(&MountTarget::Embedded).expect("serialize"),
                serde_json::from_str::<MountTarget>("\"windowed\"").expect("deserialize"),
                serde_json::to_string(&SurfaceKind::Webview).expect("serialize"),
            ),
            (
                "\"embedded\"".to_owned(),
                MountTarget::Windowed,
                "\"webview\"".to_owned(),
            )
        );
    }
}
