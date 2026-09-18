//! Runs the lifecycle commands installed Hook packages declare, and keeps this session's results.
//!
//! A Hook package ships a program that manages its own Agent integration; the host never performs
//! that integration itself. What the host owns is the boundary around running it: it resolves the
//! executable inside the installed package, re-validates it immediately before every spawn, runs
//! it in the minimal environment with no Hook Setting value, bounds its output and duration, and
//! records what happened (Hook decisions D2/D4/D5/D8).
//!
//! Execution is deliberately not part of installing. Laying a package down and running a program
//! it contains are separate authorizations, which is why a pack install runs nothing for its
//! members and why every trigger lives in an explicitly acknowledged operation.

mod api;
mod runner;
#[cfg(test)]
mod tests;

use ora_contracts::{HookLifecycleOutcome, HookLifecyclePhase, HookLifecycleReport};
use ora_logging::{ora_info, ora_warn};
use ora_plugin_config::HookLifecycleCommand;
use ora_plugin_manager::{InstalledHookDescriptor, validate_executable_containment};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

pub(crate) use runner::{
    HOOK_COMMAND_TIMEOUT, HookCommandFinished, HookCommandOutput, HookCommandSpec,
};
// The lifecycle operations are tested from their own module, which substitutes the same runner this
// module's tests do, so the rest of the run contract is crate-visible under `cfg(test)` and never
// crosses the crate boundary.
#[cfg(test)]
pub(crate) use runner::{HookCommandError, HookCommandExecution, HookCommandRunner};

use runner::{HookCommandRunnerSlot, report_output};

/// The installed Hook package one execution targets.
///
/// The descriptor carries the declaration discovery validated; the package root is what
/// containment is re-checked against, because the declaration alone cannot prove the file is
/// still inside the package it named.
pub(crate) struct InstalledHook {
    pub plugin_id: String,
    pub package_root: PathBuf,
    pub descriptor: InstalledHookDescriptor,
}

/// Runs Hook lifecycle commands and remembers the last result per plugin.
///
/// The store is session-scoped by decision (D8): a result describes what this session did, and
/// the durable truth about a Hook's integration lives in the Agent configuration the tool itself
/// wrote. Persisting a copy here would create a second, weaker answer that could disagree with
/// the tool's own state.
pub(crate) struct HookLifecycle {
    /// The working directory every command runs in, and the only path the host supplies.
    home_directory: PathBuf,
    runner: HookCommandRunnerSlot,
    reports: Mutex<BTreeMap<String, HookLifecycleReport>>,
}

impl HookLifecycle {
    /// Creates the executor that runs commands in `home_directory`.
    pub(crate) fn new(home_directory: PathBuf) -> Self {
        Self {
            home_directory,
            runner: HookCommandRunnerSlot::default(),
            reports: Mutex::new(BTreeMap::new()),
        }
    }

    /// Replaces the command runner, so a test never spawns a package program.
    #[cfg(test)]
    pub(crate) fn install_runner(&self, runner: impl HookCommandRunner) {
        self.runner.install(runner);
    }

    /// Runs one lifecycle phase for `hook` and records the result.
    ///
    /// Returns `None` when the package declares no command for the phase, which only `deinit` can
    /// do. A command that fails is still recorded and returned rather than propagated: a failed
    /// lifecycle command does not change installation state (D8), so the caller has nothing to
    /// roll back — the report is what makes the failure visible and the phase retryable.
    pub(crate) async fn execute(
        &self,
        hook: &InstalledHook,
        phase: HookLifecyclePhase,
    ) -> Option<HookLifecycleReport> {
        let lifecycle = &hook.descriptor.configuration.hook.lifecycle;
        let command = match phase {
            HookLifecyclePhase::Init => Some(&lifecycle.init),
            HookLifecyclePhase::Deinit => lifecycle.deinit.as_ref(),
        };
        let command = command?;
        let executable = hook
            .descriptor
            .configuration
            .hook
            .executable
            .as_str()
            .to_owned();
        let (outcome, output) = self.attempt(hook, command).await;
        let output = report_output(&output);
        let report = HookLifecycleReport {
            plugin_id: hook.plugin_id.clone(),
            phase,
            executable,
            outcome,
            output: output.text,
            output_truncated: output.truncated,
        };
        self.record(&report);
        Some(report)
    }

    /// Returns one result per plugin this session has run a command for, in identifier order.
    pub(crate) fn reports(&self) -> Vec<HookLifecycleReport> {
        self.reports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Spawns the declared command once and classifies how the attempt ended.
    async fn attempt(
        &self,
        hook: &InstalledHook,
        command: &HookLifecycleCommand,
    ) -> (HookLifecycleOutcome, HookCommandOutput) {
        let descriptor = &hook.descriptor.configuration.hook;
        // Containment is re-checked against the installed package rather than trusted from
        // discovery: the package sits on disk between the two reads, and the file the host runs
        // must be the file it just proved is inside the package.
        let program = match validate_executable_containment(&hook.package_root, descriptor) {
            Ok(program) => program,
            Err(error) => {
                return (
                    failed(
                        None,
                        0,
                        format!(
                            "the installed package no longer resolves its declared executable: {error}"
                        ),
                    ),
                    HookCommandOutput::default(),
                );
            }
        };
        let spec = HookCommandSpec {
            program,
            args: command.args().to_vec(),
            working_directory: self.home_directory.clone(),
        };
        let started = Instant::now();
        let execution = match self.runner.run(&spec).await {
            Ok(execution) => execution,
            Err(error) => {
                return (
                    failed(None, elapsed_millis(started), error.message),
                    HookCommandOutput::default(),
                );
            }
        };
        let duration = elapsed_millis(started);
        let outcome = match execution.finished {
            HookCommandFinished::TimedOut => failed(
                None,
                duration,
                format!(
                    "the command was still running after {} seconds and was terminated",
                    HOOK_COMMAND_TIMEOUT.as_secs()
                ),
            ),
            HookCommandFinished::Exited { code: Some(0) } => HookLifecycleOutcome::Succeeded {
                duration_ms: duration,
            },
            HookCommandFinished::Exited { code } => {
                let reason = match code {
                    Some(code) => format!("the command exited with code {code}"),
                    None => "the command was terminated before it exited".to_string(),
                };
                failed(code, duration, reason)
            }
        };
        (outcome, execution.stderr)
    }

    /// Stores one result and logs it, so a session's executions are inspectable twice over.
    fn record(&self, report: &HookLifecycleReport) {
        let phase = phase_name(report.phase);
        match &report.outcome {
            HookLifecycleOutcome::Succeeded { duration_ms } => ora_info!(
                plugin_id = %report.plugin_id,
                phase,
                duration_ms,
                "Hook lifecycle command succeeded"
            ),
            HookLifecycleOutcome::Failed {
                exit_code,
                duration_ms,
                reason,
            } => ora_warn!(
                plugin_id = %report.plugin_id,
                phase,
                duration_ms,
                exit_code = exit_code.map(|code| code.to_string()).unwrap_or_default(),
                reason = %reason,
                output = %report.output,
                "Hook lifecycle command failed"
            ),
        }
        self.reports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(report.plugin_id.clone(), report.clone());
    }
}

/// Builds a failed outcome whose reason already explains the attempt.
fn failed(exit_code: Option<i32>, duration_ms: u32, reason: String) -> HookLifecycleOutcome {
    HookLifecycleOutcome::Failed {
        exit_code,
        duration_ms,
        reason,
    }
}

/// Renders the elapsed time as the whole milliseconds a report carries.
fn elapsed_millis(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// Names one phase for the structured logs.
fn phase_name(phase: HookLifecyclePhase) -> &'static str {
    match phase {
        HookLifecyclePhase::Init => "init",
        HookLifecyclePhase::Deinit => "deinit",
    }
}
