//! Hook lifecycle result types: the phases the host executes, the session's per-plugin result,
//! and the explicit initialization request.
//!
//! A Hook package's declared commands run outside Ora's plugin sandbox, so the host only
//! authorizes, executes, records, and prompts for a restart (Hook decision D4/D5). These types
//! carry the part of that flow a caller can observe: which phase ran, how it ended, and — for a
//! failure — why. They are deliberately not persisted: a result describes what this session did,
//! and a later session asks the package's own Agent configuration for the durable truth (D8).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Names one lifecycle phase a Hook package declares a command for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum HookLifecyclePhase {
    /// Runs after an authorized install, import, or update commits.
    Init,
    /// Runs after an authorized uninstall is confirmed, while the package is still on disk.
    Deinit,
}

/// Reports how one lifecycle execution ended.
///
/// The two states are a closed enum rather than a success flag plus optional error text so a
/// caller cannot observe a failure without a reason to show the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum HookLifecycleOutcome {
    /// The command ran to completion and exited successfully.
    Succeeded {
        /// Wall-clock time the command took, so a slow tool is visible without reading logs.
        duration_ms: u32,
    },
    /// The command did not complete successfully: it exited non-zero, was killed after exceeding
    /// the host's time limit, or could not be started at all.
    ///
    /// The failure never changes installation state (D8): the package stays installed and the
    /// user can retry the phase after fixing whatever the command reported.
    Failed {
        /// The process exit code, absent when the command was killed or never started.
        exit_code: Option<i32>,
        /// Wall-clock time the attempt took, including the timeout that ended it.
        duration_ms: u32,
        /// Why the attempt failed, shown to the user so retrying is an informed decision.
        reason: String,
    },
}

/// Describes the most recent lifecycle execution the host performed for one installed package.
///
/// The result is keyed by plugin rather than by phase: an update re-runs `init`, and an uninstall
/// runs `deinit` immediately before the package disappears, so only the latest execution is ever
/// meaningful to a reader. A Hook the host has never executed for has no report at all, which is
/// how a pack-installed member reports "not initialized" without the pack path having to run
/// anything (D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct HookLifecycleReport {
    pub plugin_id: String,
    pub phase: HookLifecyclePhase,
    /// The package-relative executable that ran, so the result stays tied to the disclosure the
    /// user authorized even if a later version of the package ships a different one.
    pub executable: String,
    pub outcome: HookLifecycleOutcome,
    /// The command's captured `stderr`, bounded by the host's capture limit and trimmed.
    ///
    /// A tool that refuses to install its Hook explains why on its error stream, so the failure
    /// stays actionable in the UI instead of only in the host logs. Empty when the command wrote
    /// nothing or never started.
    pub output: String,
    /// Whether `output` was cut at the host's capture limit.
    pub output_truncated: bool,
}

/// Requests this session's Hook lifecycle results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListHookLifecycleReportsRequest {}

/// Returns one result per Hook package this session has executed a command for.
///
/// Packages with no result are absent rather than reported as a state of their own: the caller
/// distinguishes "never ran" from "ran and failed" by presence, which is the same distinction the
/// host makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListHookLifecycleReportsResponse {
    pub reports: Vec<HookLifecycleReport>,
}

/// Requests the `init` lifecycle command of one installed Hook package.
///
/// This is the retry path for a failed `init` and the only way a pack-installed Hook member is
/// ever initialized: a pack install lands packages without running anything (D2), so the user
/// authorizes each member individually afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InitializeHookRequest {
    pub plugin_id: String,
    /// Declares that the user authorized this execution; see
    /// [`InstallPluginRequest::hook_execution_acknowledged`](crate::InstallPluginRequest).
    #[serde(default)]
    pub hook_execution_acknowledged: bool,
}

/// Returns the result of the initialization the caller asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InitializeHookResponse {
    pub report: HookLifecycleReport,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    HookLifecyclePhase::export(config)?;
    HookLifecycleOutcome::export(config)?;
    HookLifecycleReport::export(config)?;
    ListHookLifecycleReportsRequest::export(config)?;
    ListHookLifecycleReportsResponse::export(config)?;
    InitializeHookRequest::export(config)?;
    InitializeHookResponse::export(config)?;
    Ok(())
}
