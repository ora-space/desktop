//! Behaviour of the Hook lifecycle executor in isolation from the rest of the plugin host.
//!
//! Every case drives the production executor and substitutes only the command runner, so what is
//! under test is the boundary the host owns — which command it resolves, what it passes to it, and
//! what it records — rather than a package program's own behaviour.

use super::runner::MAX_HOOK_REPORT_OUTPUT_BYTES;
use super::{
    HOOK_COMMAND_TIMEOUT, HookCommandError, HookCommandExecution, HookCommandFinished,
    HookCommandOutput, HookCommandRunner, HookCommandSpec, HookLifecycle, InstalledHook,
};
use ora_contracts::{HookLifecycleOutcome, HookLifecyclePhase, HookLifecycleReport};
use ora_plugin_config::compile_hook_configuration_from_bytes;
use ora_plugin_manager::InstalledHookDescriptor;
use pretty_assertions::assert_eq;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// The Hook declaration these cases execute: both phases, with arguments worth observing.
const HOOK_CONFIG: &[u8] = br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init","--scope","user"]},"deinit":{"args":["--deinit"]}}}}"#;

/// A Hook declaration with no removal command, which is the shape that skips `deinit`.
const INIT_ONLY_CONFIG: &[u8] = br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init"]}}}}"#;

/// One command the executor resolved, holding everything the runner was asked to do.
#[derive(Debug, PartialEq, Eq)]
struct RecordedCall {
    program: PathBuf,
    args: Vec<String>,
    working_directory: PathBuf,
}

/// How a [`FakeRunner`] answers one call.
enum Reply {
    /// Reports a process that exited with `code` after writing `stderr`.
    Exit { code: i32, stderr: String },
    /// Reports a process the host killed once the time limit elapsed.
    TimedOut,
    /// Reports that the host could not start a process at all.
    StartFailure(String),
}

impl Reply {
    /// Reports a process that exited with `code` and wrote nothing to standard error.
    fn exit(code: i32) -> Self {
        Self::Exit {
            code,
            stderr: String::new(),
        }
    }
}

/// Answers every lifecycle command with a scripted result and remembers what it was asked to run.
///
/// The executor resolves and validates the command itself, so a case that wants to know what would
/// have run has to observe the spec rather than the package: this runner is the only place the
/// resolved program, its arguments, and its working directory are visible without a real process.
struct FakeRunner {
    calls: Mutex<Vec<RecordedCall>>,
    reply: Reply,
}

impl FakeRunner {
    /// Builds a runner that answers every call with `reply`.
    fn new(reply: Reply) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply,
        }
    }

    /// Returns the calls recorded so far, in order.
    fn calls(&self) -> Vec<RecordedCall> {
        std::mem::take(&mut *self.calls.lock().expect("calls lock"))
    }
}

impl HookCommandRunner for Arc<FakeRunner> {
    /// Records the resolved spec and answers with the scripted reply.
    async fn run(&self, spec: &HookCommandSpec) -> Result<HookCommandExecution, HookCommandError> {
        self.calls.lock().expect("calls lock").push(RecordedCall {
            program: spec.program.clone(),
            args: spec.args.clone(),
            working_directory: spec.working_directory.clone(),
        });
        match &self.reply {
            Reply::Exit { code, stderr } => Ok(HookCommandExecution {
                finished: HookCommandFinished::Exited { code: Some(*code) },
                stderr: HookCommandOutput {
                    text: stderr.clone(),
                    truncated: false,
                },
            }),
            Reply::TimedOut => Ok(HookCommandExecution {
                finished: HookCommandFinished::TimedOut,
                stderr: HookCommandOutput::default(),
            }),
            Reply::StartFailure(message) => Err(HookCommandError {
                message: message.clone(),
            }),
        }
    }
}

/// Runs one asynchronous case under the TRACE subscriber and clock every execution needs.
///
/// The body is driven by a current-thread runtime on this test's own thread, so the scoped
/// subscriber covers the logs an execution emits rather than only the fixture that precedes it.
fn run_lifecycle<Body>(body: Body) -> Body::Output
where
    Body: Future,
{
    ora_logging::initialize_test_clock();
    ora_logging::with_trace_logging(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| panic!("expected a tokio runtime: {error}"))
            .block_on(body)
    })
}

/// Writes the package layout the declarations above describe and returns its root.
fn write_hook_package(config: &[u8]) -> TempDir {
    let root = TempDir::new().expect("package root");
    let assets = root.path().join("assets");
    std::fs::create_dir_all(&assets).expect("create assets");
    std::fs::write(root.path().join("assets/config.json"), config).expect("write configuration");
    std::fs::write(assets.join("rtk.exe"), b"MZdummy").expect("write executable");
    root
}

/// Returns the canonical path the host resolves for the package's declared executable.
fn executable_path(package_root: &Path) -> PathBuf {
    package_root
        .canonicalize()
        .expect("canonical package root")
        .join("assets")
        .join("rtk.exe")
}

/// Builds one installed Hook from a package root and its `assets/config.json` bytes.
fn installed_hook(root: &Path, config: &[u8]) -> InstalledHook {
    InstalledHook {
        plugin_id: "official/rtk".to_string(),
        package_root: root.to_path_buf(),
        descriptor: InstalledHookDescriptor {
            configuration: compile_hook_configuration_from_bytes(config)
                .expect("compile Hook configuration"),
            artifact_target: None,
        },
    }
}

/// Erases the wall-clock duration a report measured, which no assertion can predict.
fn without_duration(report: &HookLifecycleReport) -> HookLifecycleReport {
    let outcome = match &report.outcome {
        HookLifecycleOutcome::Succeeded { .. } => {
            HookLifecycleOutcome::Succeeded { duration_ms: 0 }
        }
        HookLifecycleOutcome::Failed {
            exit_code, reason, ..
        } => HookLifecycleOutcome::Failed {
            exit_code: *exit_code,
            duration_ms: 0,
            reason: reason.clone(),
        },
    };
    HookLifecycleReport {
        outcome,
        ..report.clone()
    }
}

/// The declared arguments reach the resolved package executable, which the host runs in its own
/// directory rather than in the package.
#[test]
fn runs_the_declared_command_in_the_host_directory() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(HOOK_CONFIG);
    let runner = Arc::new(FakeRunner::new(Reply::exit(0)));
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::clone(&runner));

    let report = run_lifecycle(lifecycle.execute(
        &installed_hook(package.path(), HOOK_CONFIG),
        HookLifecyclePhase::Init,
    ))
    .expect("init is declared");

    assert_eq!(
        runner.calls(),
        vec![RecordedCall {
            program: executable_path(package.path()),
            args: vec![
                "--init".to_string(),
                "--scope".to_string(),
                "user".to_string()
            ],
            working_directory: home.path().to_path_buf(),
        }],
        "the host runs the resolved package executable with the declared arguments"
    );
    assert_eq!(
        without_duration(&report),
        HookLifecycleReport {
            plugin_id: "official/rtk".to_string(),
            phase: HookLifecyclePhase::Init,
            executable: "assets/rtk.exe".to_string(),
            outcome: HookLifecycleOutcome::Succeeded { duration_ms: 0 },
            output: String::new(),
            output_truncated: false,
        }
    );
}

/// A non-zero exit is a recorded failure, not a host error: the caller has no install state to
/// roll back, and the report is what makes the phase visible and retryable.
#[test]
fn records_a_non_zero_exit_as_a_failure_with_the_code() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(INIT_ONLY_CONFIG);
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::new(FakeRunner::new(Reply::exit(3))));

    let report = run_lifecycle(lifecycle.execute(
        &installed_hook(package.path(), INIT_ONLY_CONFIG),
        HookLifecyclePhase::Init,
    ))
    .expect("init is declared");

    assert_eq!(
        without_duration(&report),
        HookLifecycleReport {
            plugin_id: "official/rtk".to_string(),
            phase: HookLifecyclePhase::Init,
            executable: "assets/rtk.exe".to_string(),
            outcome: HookLifecycleOutcome::Failed {
                exit_code: Some(3),
                duration_ms: 0,
                reason: "the command exited with code 3".to_string(),
            },
            output: String::new(),
            output_truncated: false,
        }
    );
    assert_eq!(
        lifecycle.reports(),
        vec![report],
        "a failed execution is still remembered so the surface can show it"
    );
}

/// A command the host kills at the time limit is reported with the limit rather than with an
/// invented exit code.
#[test]
fn reports_a_command_that_exceeded_the_time_limit() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(INIT_ONLY_CONFIG);
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::new(FakeRunner::new(Reply::TimedOut)));

    let report = run_lifecycle(lifecycle.execute(
        &installed_hook(package.path(), INIT_ONLY_CONFIG),
        HookLifecyclePhase::Init,
    ))
    .expect("init is declared");

    assert_eq!(
        without_duration(&report),
        HookLifecycleReport {
            plugin_id: "official/rtk".to_string(),
            phase: HookLifecyclePhase::Init,
            executable: "assets/rtk.exe".to_string(),
            outcome: HookLifecycleOutcome::Failed {
                exit_code: None,
                duration_ms: 0,
                reason: format!(
                    "the command was still running after {} seconds and was terminated",
                    HOOK_COMMAND_TIMEOUT.as_secs()
                ),
            },
            output: String::new(),
            output_truncated: false,
        }
    );
}

/// A command the host cannot start is reported as a failed attempt instead of a spawn error that
/// would surface as a failure of the install the user asked for.
#[test]
fn reports_a_command_that_could_not_start() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(INIT_ONLY_CONFIG);
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::new(FakeRunner::new(Reply::StartFailure(
        "failed to start `rtk.exe`: access denied".to_string(),
    ))));

    let report = run_lifecycle(lifecycle.execute(
        &installed_hook(package.path(), INIT_ONLY_CONFIG),
        HookLifecyclePhase::Init,
    ))
    .expect("init is declared");

    assert_eq!(
        without_duration(&report),
        HookLifecycleReport {
            plugin_id: "official/rtk".to_string(),
            phase: HookLifecyclePhase::Init,
            executable: "assets/rtk.exe".to_string(),
            outcome: HookLifecycleOutcome::Failed {
                exit_code: None,
                duration_ms: 0,
                reason: "failed to start `rtk.exe`: access denied".to_string(),
            },
            output: String::new(),
            output_truncated: false,
        }
    );
}

/// The package's own error stream reaches the report, bounded, so a failure is actionable without
/// reading the host logs.
#[test]
fn bounds_the_output_a_report_carries() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(INIT_ONLY_CONFIG);
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::new(FakeRunner::new(Reply::Exit {
        code: 1,
        stderr: "e".repeat(MAX_HOOK_REPORT_OUTPUT_BYTES * 2),
    })));

    let report = run_lifecycle(lifecycle.execute(
        &installed_hook(package.path(), INIT_ONLY_CONFIG),
        HookLifecyclePhase::Init,
    ))
    .expect("init is declared");

    assert_eq!(
        (report.output, report.output_truncated),
        ("e".repeat(MAX_HOOK_REPORT_OUTPUT_BYTES), true),
        "the report carries the bounded head of the error stream"
    );
}

/// Discovery proved the executable was inside the package; the executor proves it again before
/// every spawn, because the package sits on disk in between.
#[test]
fn re_validates_containment_before_every_spawn() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(INIT_ONLY_CONFIG);
    std::fs::remove_file(package.path().join("assets").join("rtk.exe")).expect("remove executable");
    let runner = Arc::new(FakeRunner::new(Reply::exit(0)));
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::clone(&runner));

    let report = run_lifecycle(lifecycle.execute(
        &installed_hook(package.path(), INIT_ONLY_CONFIG),
        HookLifecyclePhase::Init,
    ))
    .expect("init is declared");

    assert_eq!(
        runner.calls(),
        Vec::new(),
        "nothing is spawned once the executable no longer resolves inside the package"
    );
    let HookLifecycleOutcome::Failed { reason, .. } = &report.outcome else {
        panic!("expected a containment failure, got {:?}", report.outcome)
    };
    assert!(
        reason.contains("no longer resolves its declared executable"),
        "the reason names the re-check that refused the spawn, got `{reason}`"
    );
}

/// A package that declares no removal command runs nothing and records nothing on uninstall.
#[test]
fn skips_an_undeclared_deinit() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(INIT_ONLY_CONFIG);
    let runner = Arc::new(FakeRunner::new(Reply::exit(0)));
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::clone(&runner));

    let report = run_lifecycle(lifecycle.execute(
        &installed_hook(package.path(), INIT_ONLY_CONFIG),
        HookLifecyclePhase::Deinit,
    ));

    assert_eq!(report, None, "an undeclared phase has no result to report");
    assert_eq!(runner.calls(), Vec::new());
    assert_eq!(lifecycle.reports(), Vec::new());
}

/// Each phase runs with its own declared arguments, and only the latest result per plugin is kept:
/// a later phase replaces the earlier one instead of accumulating history.
#[test]
fn keeps_only_the_latest_result_per_plugin() {
    let home = TempDir::new().expect("home");
    let package = write_hook_package(HOOK_CONFIG);
    let runner = Arc::new(FakeRunner::new(Reply::exit(0)));
    let lifecycle = HookLifecycle::new(home.path().to_path_buf());
    lifecycle.install_runner(Arc::clone(&runner));
    let hook = installed_hook(package.path(), HOOK_CONFIG);

    let init = run_lifecycle(lifecycle.execute(&hook, HookLifecyclePhase::Init))
        .expect("init is declared");
    let deinit = run_lifecycle(lifecycle.execute(&hook, HookLifecyclePhase::Deinit))
        .expect("deinit is declared");

    assert_eq!(
        runner.calls(),
        vec![
            RecordedCall {
                program: executable_path(package.path()),
                args: vec![
                    "--init".to_string(),
                    "--scope".to_string(),
                    "user".to_string()
                ],
                working_directory: home.path().to_path_buf(),
            },
            RecordedCall {
                program: executable_path(package.path()),
                args: vec!["--deinit".to_string()],
                working_directory: home.path().to_path_buf(),
            },
        ]
    );
    assert_eq!(
        (init.phase, deinit.phase),
        (HookLifecyclePhase::Init, HookLifecyclePhase::Deinit)
    );
    assert_eq!(
        lifecycle.reports(),
        vec![deinit],
        "the store answers for the plugin, not for a phase"
    );
}
