use serde::Deserialize;

/// Mirrors the package fields required by Ora without rejecting unrelated npm metadata.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageManifest {
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub package_type: String,
    pub ora: OraManifest,
}

/// Mirrors version-one Ora plugin metadata from `package.json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OraManifest {
    pub manifest_version: u32,
    pub id: String,
    pub display_name: String,
    pub kind: String,
    pub main: String,
    pub engines: EngineManifest,
    pub contributes: ContributionManifest,
}

/// Mirrors engine declarations while leaving npm-style ranges uninterpreted.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EngineManifest {
    pub ora: String,
    pub plugin_api: u32,
    pub bun: String,
}

/// Mirrors contributions declared by one plugin package.
///
/// Every contribution is optional here because the required set depends on `ora.kind`; semantic
/// validation resolves which one this package had to declare.
#[derive(Debug, Deserialize)]
pub(crate) struct ContributionManifest {
    pub agent: Option<AgentManifest>,
    pub ui: Option<UiManifest>,
}

/// Mirrors the single agent an agent-kind package contributes.
///
/// The agent carries no id of its own: one package provides exactly one agent, whose identity is
/// the package's `ora.id`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentManifest {
    pub display_name: String,
    pub contract_version: u32,
}

/// Mirrors the `contributes.ui` block of a ui-kind package.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiManifest {
    pub contract_version: u32,
    pub surfaces: Vec<SurfaceManifest>,
}

/// Mirrors one declared surface before semantic validation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SurfaceManifest {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub instance_policy: InstancePolicyManifest,
    pub source: SurfaceSourceManifest,
}

/// Mirrors the `instancePolicy` spelling; `singleton` is the only value and the default.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum InstancePolicyManifest {
    #[default]
    Singleton,
}

/// Mirrors the tagged surface source union.
///
/// Unknown `kind` values fail structurally during deserialization, which surfaces as an invalid
/// JSON issue with the `kind` field path rather than a semantic one.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum SurfaceSourceManifest {
    #[serde(rename_all = "camelCase")]
    RemoteSite {
        entry_url: String,
        navigation: NavigationManifest,
        #[serde(default)]
        web_data: WebDataPolicyManifest,
    },
    /// A page shipped inside the plugin package, served from `root` starting at `entry`.
    #[serde(rename_all = "camelCase")]
    Panel { root: String, entry: String },
}

/// Mirrors the navigation allow lists; both lists are optional and combined by union.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationManifest {
    #[serde(default)]
    pub allow_hosts: Vec<String>,
    #[serde(default)]
    pub allow_host_suffixes: Vec<String>,
}

/// Mirrors the `webData` spelling; a persistent profile is the default.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WebDataPolicyManifest {
    #[default]
    PersistentProfile,
    EphemeralIsolated,
}
