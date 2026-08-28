use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Describes the kind-specific contribution of one installed plugin, discriminated by `kind`.
///
/// The agent variant names its display name `agentDisplayName` because the contribution is
/// flattened into [`InstalledPlugin`], which already owns the top-level `displayName`. The two
/// surface kinds expose only what the launcher needs to render an entry: the frontend never
/// learns asset paths, origin allow lists, or download rules, which is what keeps those host
/// policies non-negotiable from the page side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum InstalledPluginContribution {
    Agent {
        agent_display_name: String,
    },
    /// A page shipped inside the package and bridged to the plugin's own process.
    Workbench {
        title: String,
    },
    /// An external HTTPS site shown in an isolated webview; `start_url` is informational only.
    Webview {
        title: String,
        start_url: String,
    },
    /// A static package kind whose Skill assets are cataloged without a runtime process.
    Skill,
    /// A configuration-only kind describing one MCP Server; transport details stay host-side.
    Mcp,
    /// A processless Hook contribution: one immutable Hook Protocol descriptor and one
    /// package-contained executable. The frontend never learns the executable path; it renders
    /// the protocol, command alias, target, and embedded tool version for audit.
    Hook {
        protocol: String,
        command: String,
        /// The target triple the installed physical artifact self-declares, absent for a
        /// universal release.
        target: Option<String>,
        /// The embedded tool version, independent from the Hook Plugin version.
        tool_version: String,
    },
}

/// Represents whether the installed package and its immutable declaration are usable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "validity",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum PluginInstallationValidity {
    Valid,
    InvalidDeclaration { error_code: String },
}

/// Reports whether every required Setting has an effective type-correct value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginConfigurationCompleteness {
    Complete,
    Incomplete,
}

/// Represents the exclusive list-facing Plugin Configuration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum PluginConfigurationSummary {
    NotDeclared,
    Available {
        completeness: PluginConfigurationCompleteness,
    },
    Unavailable {
        error_code: String,
    },
}

/// Enumerates Setting types supported by declaration schema version one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginSettingType {
    String,
    Number,
    Boolean,
}

/// Carries one non-secret scalar override accepted by schema version one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "plugin.ts")]
pub enum PluginSettingValue {
    String(String),
    Number(f64),
    Boolean(bool),
}

/// Describes one immutable plugin-authored Setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PluginSettingDeclaration {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub setting_type: PluginSettingType,
    pub required: bool,
    pub order: Option<i64>,
    pub default: Option<PluginSettingValue>,
}

/// Identifies the source of one effective editor value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginSettingValueSource {
    Stored,
    Default,
    Absent,
}

/// Projects one Setting into an editor field without exposing raw files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PluginSettingDetails {
    pub declaration: PluginSettingDeclaration,
    pub stored_value: Option<PluginSettingValue>,
    pub effective_value: Option<PluginSettingValue>,
    pub source: PluginSettingValueSource,
    pub value_error_code: Option<String>,
}

/// Carries one complete editor snapshot bound to a revision and declaration fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PluginConfigurationDetails {
    pub plugin_id: String,
    pub schema_version: u32,
    pub revision: u64,
    pub declaration_fingerprint: String,
    pub settings: Vec<PluginSettingDetails>,
    pub summary: PluginConfigurationSummary,
}

/// Represents the process-scoped lifecycle of one installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "runtime",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum PluginRuntimeStatus {
    Stopped,
    Starting,
    Running,
    Failed { failure_reason: String },
}

/// Describes one installed plugin discovered from its `orax.toml` manifest.
///
/// `id` is the canonical `<namespace>/<name>` spelling and is what every plugin request carries
/// back; `namespace` and `name` repeat the two segments so the frontend never has to split it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstalledPlugin {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub homepage: Option<String>,
    pub license: Option<String>,
    #[serde(flatten)]
    #[ts(flatten)]
    pub contribution: InstalledPluginContribution,
    /// Security-validated SVG source for the package icon, absent when the package ships none.
    ///
    /// The icon travels as inline source instead of a filesystem path because the webview cannot
    /// read the plugin directory; surfaces render it from a `data:` URL and fall back to a
    /// generic mark when it is absent.
    pub logo: Option<String>,
    pub installation_validity: PluginInstallationValidity,
    pub configuration: PluginConfigurationSummary,
    #[serde(flatten)]
    #[ts(flatten)]
    pub runtime: PluginRuntimeStatus,
}

/// Describes one marketplace plugin listed by the cached registry index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct AvailablePlugin {
    pub id: String,
    pub name: String,
    /// Human-readable display title declared by the manifest; falls back to `name` when a cached
    /// index or older manifest omits it.
    pub title: String,
    /// The plugin kind (`agent`, `workbench`, `webview`, `skill`, `mcp`, or `hook`).
    pub kind: String,
    pub namespace: String,
    pub version: String,
    pub description: String,
    /// Security-validated SVG source for the marketplace icon, absent when none is published.
    pub logo: Option<String>,
    /// Host compatibility as a closed enum so a listing cannot be both compatible and carry a
    /// reason, or incompatible without one.
    #[serde(flatten)]
    #[ts(flatten)]
    pub compatibility: PluginHostCompatibility,
}

/// Reports whether the current host can install one marketplace listing.
///
/// A universal release is always compatible. A targeted release is compatible only when the host
/// target has a matching artifact. A listing with no downloadable release is incompatible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "compatibility",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum PluginHostCompatibility {
    Compatible,
    Incompatible { reason: String },
}

/// Requests the cached marketplace registry index used to populate the plugin catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListAvailablePluginsRequest {}

/// Returns the marketplace plugins cached in the registry index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListAvailablePluginsResponse {
    pub updated_at: i64,
    pub plugins: Vec<AvailablePlugin>,
}

/// Requests a marketplace source sync followed by an atomic registry-index rebuild.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SyncAvailablePluginsRequest {}

/// Returns the registry index rebuilt immediately after a marketplace sync succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SyncAvailablePluginsResponse {
    pub updated_at: i64,
    pub plugins: Vec<AvailablePlugin>,
}

/// Requests the README one marketplace listing publishes beside its `orax.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ReadPluginReadmeRequest {
    /// The canonical `namespace/name` marketplace identifier.
    pub plugin_id: String,
}

/// Returns the README text the winning marketplace source publishes, absent when none ships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ReadPluginReadmeResponse {
    pub readme: Option<String>,
}

/// Lists one configured marketplace source repository and its tracked branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct MarketplaceSource {
    /// HTTPS Git repository URL of the marketplace checkout.
    pub url: String,
    /// Short branch name tracked by the source.
    pub branch: String,
    /// Whether Git fetches and plugin downloads for this source use the configured proxy.
    pub use_proxy: bool,
}

/// Requests the configured marketplace source repositories.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListMarketplaceSourcesRequest {}

/// Returns every configured marketplace source in source-precedence order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListMarketplaceSourcesResponse {
    pub sources: Vec<MarketplaceSource>,
}

/// Requests adding one marketplace Git source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct AddMarketplaceSourceRequest {
    pub url: String,
    pub branch: String,
    pub use_proxy: bool,
}

/// Returns the source list immediately after one source is persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct AddMarketplaceSourceResponse {
    pub sources: Vec<MarketplaceSource>,
}

/// Requests changing only one marketplace source's proxy policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UpdateMarketplaceSourceRequest {
    pub url: String,
    pub use_proxy: bool,
}

/// Returns the source list immediately after one source's proxy policy is persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UpdateMarketplaceSourceResponse {
    pub sources: Vec<MarketplaceSource>,
}

/// Requests removal of one marketplace Git source by its URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct DeleteMarketplaceSourceRequest {
    pub url: String,
}

/// Returns the source list immediately after one source is removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct DeleteMarketplaceSourceResponse {
    pub sources: Vec<MarketplaceSource>,
}

/// Requests the immutable startup snapshot of installed plugin packages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListInstalledPluginsRequest {}

/// Returns every valid installed plugin in stable identifier order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListInstalledPluginsResponse {
    pub plugins: Vec<InstalledPlugin>,
}

/// Requests explicit filesystem discovery and state reconciliation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ScanPluginsRequest {}

/// Returns the refreshed installed-plugin snapshot produced by an explicit scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ScanPluginsResponse {
    pub plugins: Vec<InstalledPlugin>,
}

/// Requests process activation for one installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ActivatePluginRequest {
    pub plugin_id: String,
}

/// Returns the immediate starting or already-running plugin snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ActivatePluginResponse {
    pub plugin: InstalledPlugin,
}

/// Requests process shutdown while leaving the installed plugin available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct StopPluginRequest {
    pub plugin_id: String,
}

/// Returns the stopped plugin snapshot after process exit is confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct StopPluginResponse {
    pub plugin: InstalledPlugin,
}

/// Requests complete removal of one plugin package and its durable lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UninstallPluginRequest {
    pub plugin_id: String,
    pub data_disposition: PluginDataDisposition,
}

/// Selects whether uninstall retains or deletes host-owned plugin data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginDataDisposition {
    Delete,
    Retain,
}

/// Confirms the identifier removed after process shutdown and package deletion complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UninstallPluginResponse {
    pub plugin_id: String,
}

/// Requests installation of one marketplace plugin by its registry identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstallPluginRequest {
    pub plugin_id: String,
}

/// Confirms the identifier installed after download, verification, and extraction complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstallPluginResponse {
    pub plugin_id: String,
    /// The typed installation outcome. Installation always retains the package. A conflict-free
    /// install reports `installed`; a Hook whose command alias collides with another installed
    /// Hook reports `installed_with_command_conflict` carrying the colliding identity. Both
    /// packages remain available: the host has no enablement state, and uniqueness is deferred
    /// to a future consumer.
    pub outcome: InstallOutcome,
}

/// Models the closed set of installation outcomes.
///
/// The outcome is a closed enum rather than a pair of booleans so a caller can never observe
/// contradictory success flags. Installation always succeeds and the package remains available;
/// a command-alias collision is reported rather than silently sharing a PATH alias or pretending
/// the new package was disabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum InstallOutcome {
    /// The package was installed and is available.
    Installed,
    /// The package was installed and remains available, but another installed Hook already owns
    /// the same command alias. The colliding plugin identity is carried so a future consumer can
    /// refuse ambiguous PATH resolution instead of silently selecting the wrong Hook.
    InstalledWithCommandConflict { conflict_plugin_id: String },
}

/// Requests updating one installed marketplace plugin to the version its source publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UpdatePluginRequest {
    pub plugin_id: String,
}

/// Confirms the identifier updated after the new release is verified and stale versions removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UpdatePluginResponse {
    pub plugin_id: String,
}

/// Requests importing one local `.orax` release archive into the installed plugins tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ImportPluginRequest {
    /// Absolute path to the local `.orax` archive.
    pub path: String,
}

/// Confirms the identifier imported after the archive is verified and extracted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ImportPluginResponse {
    pub plugin_id: String,
    /// The typed installation outcome, identical in shape to a marketplace install.
    pub outcome: InstallOutcome,
}

/// Requests the current editor snapshot for one installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct GetPluginConfigurationRequest {
    pub plugin_id: String,
}

/// Returns the resolved editor snapshot without exposing its filesystem location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct GetPluginConfigurationResponse {
    pub configuration: PluginConfigurationDetails,
}

/// Replaces every explicit override recognized by the loaded declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SavePluginConfigurationRequest {
    pub plugin_id: String,
    pub expected_revision: u64,
    pub declaration_fingerprint: String,
    pub values: BTreeMap<String, PluginSettingValue>,
}

/// Returns the authoritative post-save editor snapshot and list summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SavePluginConfigurationResponse {
    pub configuration: PluginConfigurationDetails,
}

/// Selects the explicit reset operation authorized by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum ResetPluginConfigurationMode {
    ResetAll { expected_revision: u64 },
    RecoverCorrupt,
}

/// Requests Reset All or confirmed damaged-data recovery for one plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ResetPluginConfigurationRequest {
    pub plugin_id: String,
    pub declaration_fingerprint: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub reset: ResetPluginConfigurationMode,
}

/// Returns the authoritative editor snapshot after a reset operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ResetPluginConfigurationResponse {
    pub configuration: PluginConfigurationDetails,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    InstalledPluginContribution::export(config)?;
    PluginInstallationValidity::export(config)?;
    PluginConfigurationCompleteness::export(config)?;
    PluginConfigurationSummary::export(config)?;
    PluginSettingType::export(config)?;
    PluginSettingValue::export(config)?;
    PluginSettingDeclaration::export(config)?;
    PluginSettingValueSource::export(config)?;
    PluginSettingDetails::export(config)?;
    PluginConfigurationDetails::export(config)?;
    PluginRuntimeStatus::export(config)?;
    InstalledPlugin::export(config)?;
    PluginHostCompatibility::export(config)?;
    AvailablePlugin::export(config)?;
    ListAvailablePluginsRequest::export(config)?;
    ListAvailablePluginsResponse::export(config)?;
    SyncAvailablePluginsRequest::export(config)?;
    SyncAvailablePluginsResponse::export(config)?;
    ReadPluginReadmeRequest::export(config)?;
    ReadPluginReadmeResponse::export(config)?;
    MarketplaceSource::export(config)?;
    ListMarketplaceSourcesRequest::export(config)?;
    ListMarketplaceSourcesResponse::export(config)?;
    AddMarketplaceSourceRequest::export(config)?;
    AddMarketplaceSourceResponse::export(config)?;
    UpdateMarketplaceSourceRequest::export(config)?;
    UpdateMarketplaceSourceResponse::export(config)?;
    DeleteMarketplaceSourceRequest::export(config)?;
    DeleteMarketplaceSourceResponse::export(config)?;
    ListInstalledPluginsRequest::export(config)?;
    ListInstalledPluginsResponse::export(config)?;
    ScanPluginsRequest::export(config)?;
    ScanPluginsResponse::export(config)?;
    ActivatePluginRequest::export(config)?;
    ActivatePluginResponse::export(config)?;
    StopPluginRequest::export(config)?;
    StopPluginResponse::export(config)?;
    UninstallPluginRequest::export(config)?;
    PluginDataDisposition::export(config)?;
    UninstallPluginResponse::export(config)?;
    InstallPluginRequest::export(config)?;
    InstallOutcome::export(config)?;
    InstallPluginResponse::export(config)?;
    UpdatePluginRequest::export(config)?;
    UpdatePluginResponse::export(config)?;
    ImportPluginRequest::export(config)?;
    ImportPluginResponse::export(config)?;
    GetPluginConfigurationRequest::export(config)?;
    GetPluginConfigurationResponse::export(config)?;
    SavePluginConfigurationRequest::export(config)?;
    SavePluginConfigurationResponse::export(config)?;
    ResetPluginConfigurationMode::export(config)?;
    ResetPluginConfigurationRequest::export(config)?;
    ResetPluginConfigurationResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AddMarketplaceSourceRequest, AddMarketplaceSourceResponse, AvailablePlugin,
        DeleteMarketplaceSourceRequest, DeleteMarketplaceSourceResponse, ImportPluginRequest,
        ImportPluginResponse, InstallOutcome, InstallPluginRequest, InstallPluginResponse,
        InstalledPlugin, InstalledPluginContribution, ListAvailablePluginsRequest,
        ListAvailablePluginsResponse, ListInstalledPluginsRequest, ListInstalledPluginsResponse,
        ListMarketplaceSourcesRequest, ListMarketplaceSourcesResponse, MarketplaceSource,
        PluginConfigurationSummary, PluginInstallationValidity, PluginRuntimeStatus,
        ReadPluginReadmeRequest, ReadPluginReadmeResponse, SyncAvailablePluginsRequest,
        SyncAvailablePluginsResponse, UpdateMarketplaceSourceRequest,
        UpdateMarketplaceSourceResponse, UpdatePluginRequest, UpdatePluginResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies the installed-plugin response preserves the package manifest field mapping.
    #[test]
    fn serializes_installed_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "official/ora.claude-code".to_string(),
            namespace: "official".to_string(),
            name: "ora.claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            version: "0.1.0".to_string(),
            description: "Claude Code agent".to_string(),
            homepage: Some("https://example.com/claude-code".to_string()),
            license: Some("Apache-2.0".to_string()),
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Claude Code".to_string(),
            },
            logo: Some("<svg/>".to_string()),
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Stopped,
        };

        assert_eq!(
            serde_json::to_value(ListInstalledPluginsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ListInstalledPluginsResponse {
                plugins: vec![plugin],
            })
            .unwrap(),
            json!({
                "plugins": [{
                    "id": "official/ora.claude-code",
                    "namespace": "official",
                    "name": "ora.claude-code",
                    "displayName": "Claude Code",
                    "version": "0.1.0",
                    "description": "Claude Code agent",
                    "homepage": "https://example.com/claude-code",
                    "license": "Apache-2.0",
                    "kind": "agent",
                    "agentDisplayName": "Claude Code",
                    "logo": "<svg/>",
                    "installationValidity": { "validity": "valid" },
                    "configuration": { "state": "not_declared" },
                    "runtime": "stopped"
                }]
            })
        );
    }

    /// Verifies the two surface kinds flatten their entry metadata onto the wire object and
    /// round-trip, without exposing any host policy.
    #[test]
    fn serializes_surface_plugin_contracts() {
        let base = |name: &str, contribution: InstalledPluginContribution| InstalledPlugin {
            id: format!("official/{name}"),
            namespace: "official".to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            version: "0.1.0".to_string(),
            description: "Surface plugin".to_string(),
            homepage: None,
            license: None,
            contribution,
            logo: None,
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Stopped,
        };
        let webview = base(
            "acme.hub",
            InstalledPluginContribution::Webview {
                title: "Example Hub".to_string(),
                start_url: "https://www.example.com/".to_string(),
            },
        );
        let workbench = base(
            "acme.panel",
            InstalledPluginContribution::Workbench {
                title: "Example Panel".to_string(),
            },
        );

        let webview_value = serde_json::to_value(&webview).expect("webview plugin serializes");
        let workbench_value =
            serde_json::to_value(&workbench).expect("workbench plugin serializes");
        assert_eq!(
            (
                webview_value.get("kind"),
                webview_value.get("title"),
                webview_value.get("startUrl"),
                workbench_value.get("kind"),
                workbench_value.get("title"),
                workbench_value.get("startUrl"),
            ),
            (
                Some(&json!("webview")),
                Some(&json!("Example Hub")),
                Some(&json!("https://www.example.com/")),
                Some(&json!("workbench")),
                Some(&json!("Example Panel")),
                None,
            )
        );
        assert_eq!(
            (
                serde_json::from_value::<InstalledPlugin>(webview_value)
                    .expect("webview plugin round-trips"),
                serde_json::from_value::<InstalledPlugin>(workbench_value)
                    .expect("workbench plugin round-trips"),
            ),
            (webview, workbench)
        );
    }

    /// Verifies the static Skill contribution adds only its kind discriminator.
    #[test]
    fn serializes_skill_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "official/ora.skill-pack".to_string(),
            namespace: "official".to_string(),
            name: "ora.skill-pack".to_string(),
            display_name: "ora.skill-pack".to_string(),
            version: "0.1.1".to_string(),
            description: "Skill plugin test".to_string(),
            homepage: None,
            license: None,
            contribution: InstalledPluginContribution::Skill,
            logo: None,
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Stopped,
        };

        let value = serde_json::to_value(&plugin).expect("Skill plugin serializes");
        assert_eq!(value.get("kind"), Some(&json!("skill")));
        assert_eq!(value.as_object().map(serde_json::Map::len), Some(13));
        assert_eq!(
            serde_json::from_value::<InstalledPlugin>(value).expect("Skill plugin round-trips"),
            plugin
        );
    }

    /// Verifies the configuration-only MCP contribution adds only its kind discriminator.
    #[test]
    fn serializes_mcp_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "official/ora.tavily".to_string(),
            namespace: "official".to_string(),
            name: "ora.tavily".to_string(),
            display_name: "Tavily".to_string(),
            version: "0.1.0".to_string(),
            description: "MCP plugin test".to_string(),
            homepage: None,
            license: None,
            contribution: InstalledPluginContribution::Mcp,
            logo: None,
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Stopped,
        };

        let value = serde_json::to_value(&plugin).expect("MCP plugin serializes");
        assert_eq!(value.get("kind"), Some(&json!("mcp")));
        assert_eq!(value.as_object().map(serde_json::Map::len), Some(13));
        assert_eq!(
            serde_json::from_value::<InstalledPlugin>(value).expect("MCP plugin round-trips"),
            plugin
        );
    }

    /// Verifies an empty startup snapshot has a stable collection shape.
    #[test]
    fn serializes_empty_installed_plugin_response() {
        assert_eq!(
            serde_json::to_value(ListInstalledPluginsResponse {
                plugins: Vec::new(),
            })
            .unwrap(),
            json!({ "plugins": [] })
        );
    }

    /// Verifies the marketplace registry response carries the lightweight index metadata.
    #[test]
    fn serializes_available_plugin_response() {
        assert_eq!(
            serde_json::to_value(ListAvailablePluginsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ListAvailablePluginsResponse {
                updated_at: 1_776_244_428,
                plugins: vec![AvailablePlugin {
                    id: "official/weather".to_string(),
                    name: "weather".to_string(),
                    title: "Weather".to_string(),
                    kind: "agent".to_string(),
                    namespace: "official".to_string(),
                    version: "1.2.0".to_string(),
                    description: "Weather plugin".to_string(),
                    logo: None,
                    compatibility: super::PluginHostCompatibility::Compatible,
                }],
            })
            .unwrap(),
            json!({
                "updatedAt": 1_776_244_428,
                "plugins": [{
                    "id": "official/weather",
                    "name": "weather",
                    "title": "Weather",
                    "kind": "agent",
                    "namespace": "official",
                    "version": "1.2.0",
                    "description": "Weather plugin",
                    "logo": null,
                    "compatibility": "compatible"
                }]
            })
        );
    }

    /// Verifies the marketplace sync response mirrors the rebuilt registry index wire shape.
    #[test]
    fn serializes_sync_available_plugin_response() {
        assert_eq!(
            serde_json::to_value(SyncAvailablePluginsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(SyncAvailablePluginsResponse {
                updated_at: 1_776_244_428,
                plugins: Vec::new(),
            })
            .unwrap(),
            json!({
                "updatedAt": 1_776_244_428,
                "plugins": []
            })
        );
    }

    // TEMP-ANCHOR-MARKER
    /// Verifies the README read request/response wire shapes for the marketplace detail page.
    #[test]
    fn serializes_read_plugin_readme_contracts() {
        assert_eq!(
            serde_json::to_value(ReadPluginReadmeRequest {
                plugin_id: "official/weather".to_string(),
            })
            .unwrap(),
            json!({ "pluginId": "official/weather" })
        );
        assert_eq!(
            serde_json::to_value(ReadPluginReadmeResponse {
                readme: Some("# Weather\n\nLive forecasts.".to_string()),
            })
            .unwrap(),
            json!({ "readme": "# Weather\n\nLive forecasts." })
        );
        assert_eq!(
            serde_json::to_value(ReadPluginReadmeResponse { readme: None }).unwrap(),
            json!({ "readme": null })
        );
    }
    /// Verifies marketplace source request/response wire shapes.
    #[test]
    fn serializes_marketplace_source_contracts() {
        let source = || MarketplaceSource {
            url: "https://github.com/example/marketplace".to_string(),
            branch: "main".to_string(),
            use_proxy: false,
        };
        assert_eq!(
            serde_json::to_value(ListMarketplaceSourcesRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ListMarketplaceSourcesResponse {
                sources: vec![source()],
            })
            .unwrap(),
            json!({
                "sources": [{
                    "url": "https://github.com/example/marketplace",
                    "branch": "main",
                    "useProxy": false
                }]
            })
        );
        assert_eq!(
            serde_json::to_value(AddMarketplaceSourceRequest {
                url: "https://github.com/example/marketplace".to_string(),
                branch: "main".to_string(),
                use_proxy: false,
            })
            .unwrap(),
            json!({
                "url": "https://github.com/example/marketplace",
                "branch": "main",
                "useProxy": false
            })
        );
        assert_eq!(
            serde_json::to_value(AddMarketplaceSourceResponse {
                sources: vec![source()],
            })
            .unwrap(),
            json!({
                "sources": [{
                    "url": "https://github.com/example/marketplace",
                    "branch": "main",
                    "useProxy": false
                }]
            })
        );
        assert_eq!(
            serde_json::to_value(UpdateMarketplaceSourceRequest {
                url: "https://github.com/example/marketplace".to_string(),
                use_proxy: true,
            })
            .unwrap(),
            json!({
                "url": "https://github.com/example/marketplace",
                "useProxy": true
            })
        );
        assert_eq!(
            serde_json::to_value(UpdateMarketplaceSourceResponse {
                sources: Vec::new(),
            })
            .unwrap(),
            json!({ "sources": [] })
        );
        assert_eq!(
            serde_json::to_value(DeleteMarketplaceSourceRequest {
                url: "https://github.com/example/marketplace".to_string(),
            })
            .unwrap(),
            json!({ "url": "https://github.com/example/marketplace" })
        );
        assert_eq!(
            serde_json::to_value(DeleteMarketplaceSourceResponse {
                sources: Vec::new(),
            })
            .unwrap(),
            json!({ "sources": [] })
        );
    }

    /// Verifies the install request/response wire shape for a marketplace plugin.
    #[test]
    fn serializes_install_plugin_contract() {
        assert_eq!(
            serde_json::to_value(InstallPluginRequest {
                plugin_id: "official/weather".to_string(),
            })
            .unwrap(),
            json!({ "pluginId": "official/weather" })
        );
        assert_eq!(
            serde_json::to_value(InstallPluginResponse {
                plugin_id: "official/weather".to_string(),
                outcome: InstallOutcome::Installed,
            })
            .unwrap(),
            json!({ "pluginId": "official/weather", "outcome": { "state": "installed" } })
        );
    }

    /// Verifies the update request/response wire shape for an installed marketplace plugin.
    #[test]
    fn serializes_update_plugin_contract() {
        assert_eq!(
            serde_json::to_value(UpdatePluginRequest {
                plugin_id: "official/weather".to_string(),
            })
            .unwrap(),
            json!({ "pluginId": "official/weather" })
        );
        assert_eq!(
            serde_json::to_value(UpdatePluginResponse {
                plugin_id: "official/weather".to_string(),
            })
            .unwrap(),
            json!({ "pluginId": "official/weather" })
        );
    }

    /// Verifies the import request/response wire shape for a local `.orax` archive.
    #[test]
    fn serializes_import_plugin_contract() {
        assert_eq!(
            serde_json::to_value(ImportPluginRequest {
                path: "C:/downloads/weather.orax".to_string(),
            })
            .unwrap(),
            json!({ "path": "C:/downloads/weather.orax" })
        );
        assert_eq!(
            serde_json::to_value(ImportPluginResponse {
                plugin_id: "official/weather".to_string(),
                outcome: InstallOutcome::Installed,
            })
            .unwrap(),
            json!({ "pluginId": "official/weather", "outcome": { "state": "installed" } })
        );
    }

    /// Verifies lifecycle state is flattened into the installed-plugin wire object.
    #[test]
    fn serializes_running_plugin_lifecycle_state() {
        let plugin = InstalledPlugin {
            id: "official/ora.example".to_string(),
            namespace: "official".to_string(),
            name: "ora.example".to_string(),
            display_name: "Example".to_string(),
            version: "1.0.0".to_string(),
            description: "Example agent".to_string(),
            homepage: None,
            license: None,
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Example".to_string(),
            },
            logo: None,
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Running,
        };

        assert_eq!(
            serde_json::to_value(plugin).expect("running plugin serializes"),
            json!({
                "id": "official/ora.example",
                "namespace": "official",
                "name": "ora.example",
                "displayName": "Example",
                "version": "1.0.0",
                "description": "Example agent",
                "homepage": null,
                "license": null,
                "kind": "agent",
                "agentDisplayName": "Example",
                "logo": null,
                "installationValidity": { "validity": "valid" },
                "configuration": { "state": "not_declared" },
                "runtime": "running"
            }),
        );
    }

    /// Verifies failed runtime state carries its diagnostic reason beside the discriminator.
    #[test]
    fn serializes_failed_plugin_lifecycle_state() {
        let plugin = InstalledPlugin {
            id: "official/ora.example".to_string(),
            namespace: "official".to_string(),
            name: "ora.example".to_string(),
            display_name: "Example".to_string(),
            version: "1.0.0".to_string(),
            description: "Example agent".to_string(),
            homepage: None,
            license: None,
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Example".to_string(),
            },
            logo: None,
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Failed {
                failure_reason: "process crashed".to_string(),
            },
        };

        assert_eq!(
            serde_json::to_value(plugin).expect("failed plugin serializes"),
            json!({
                "id": "official/ora.example",
                "namespace": "official",
                "name": "ora.example",
                "displayName": "Example",
                "version": "1.0.0",
                "description": "Example agent",
                "homepage": null,
                "license": null,
                "kind": "agent",
                "agentDisplayName": "Example",
                "logo": null,
                "installationValidity": { "validity": "valid" },
                "configuration": { "state": "not_declared" },
                "runtime": "failed",
                "failureReason": "process crashed"
            }),
        );
    }
}
