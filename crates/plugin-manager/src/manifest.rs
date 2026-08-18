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
