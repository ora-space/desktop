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
    /// Deno permissions the plugin asks for; absent means fully sandboxed.
    #[serde(default)]
    pub permissions: Option<PermissionsManifest>,
}

/// Mirrors the Deno permissions a plugin declares; every field is optional and defaults to denied.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PermissionsManifest {
    #[serde(default)]
    pub run: bool,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub env: EnvPermissionManifest,
    #[serde(default)]
    pub net: bool,
}

/// Mirrors `env: true | false | ["NAME", …]` without forcing plugin authors into one spelling.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum EnvPermissionManifest {
    All(bool),
    Variables(Vec<String>),
}

impl Default for EnvPermissionManifest {
    fn default() -> Self {
        Self::All(false)
    }
}

/// Mirrors engine declarations while leaving npm-style ranges uninterpreted.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EngineManifest {
    pub ora: String,
    pub plugin_api: u32,
}

/// Mirrors contributions declared by one plugin package.
#[derive(Debug, Deserialize)]
pub(crate) struct ContributionManifest {
    pub agents: Vec<AgentManifest>,
}

/// Mirrors one contributed agent declaration.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentManifest {
    pub id: String,
    pub display_name: String,
    pub contract_version: u32,
}
