use crate::config::PluginManagerConfig;
use crate::error::PluginManagerError;
use crate::process::{PluginProcessOutput, collect_plugin_process_output};
use ora_plugin_protocol::{
    PluginAddParams, PluginJsonRpcErrorResponse, PluginJsonRpcRequest, PluginJsonRpcSuccessResponse,
};
use ora_process::{ProcessSpawner, ProcessSpec};
use std::collections::HashMap;
use std::sync::Mutex;

const ADD_PLUGIN_ID: &str = "1";
const ADD_METHOD: &str = "add";
const JSON_RPC_VERSION: &str = "2.0";

/// Describes the plugin lifecycle state visible to manager callers and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginLifecycleState {
    Registered,
    Running,
    Exited,
}

/// Routes typed application calls to the hardcoded first-slice plugin registry.
pub struct PluginManager<Spawner> {
    config: PluginManagerConfig,
    process_spawner: Spawner,
    registry: HashMap<String, PluginDefinition>,
    lifecycle_states: Mutex<HashMap<String, PluginLifecycleState>>,
}

impl<Spawner> PluginManager<Spawner> {
    /// Builds the plugin manager around a process spawner implementation.
    pub fn new(config: PluginManagerConfig, process_spawner: Spawner) -> Self {
        let registry = HashMap::from([(
            ADD_PLUGIN_ID.to_string(),
            PluginDefinition {
                id: ADD_PLUGIN_ID.to_string(),
                capabilities: vec![ADD_METHOD.to_string()],
            },
        )]);
        let lifecycle_states = Mutex::new(HashMap::from([(
            ADD_PLUGIN_ID.to_string(),
            PluginLifecycleState::Registered,
        )]));

        Self {
            config,
            process_spawner,
            registry,
            lifecycle_states,
        }
    }

    /// Returns the last observed lifecycle state for one registered plugin.
    pub fn plugin_state(&self, plugin_id: &str) -> Option<PluginLifecycleState> {
        self.lifecycle_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(plugin_id)
            .copied()
    }
}

impl<Spawner> PluginManager<Spawner>
where
    Spawner: ProcessSpawner,
{
    /// Calls the hardcoded add capability through the configured plugin process spawner.
    pub async fn number_add(&self, a: i64, b: i64) -> Result<i64, PluginManagerError> {
        let plugin = self.plugin_for_capability(ADD_PLUGIN_ID, ADD_METHOD)?;
        let plugin_id = plugin.id.clone();
        let request_id = ADD_PLUGIN_ID.to_string();
        let rpc_request = PluginJsonRpcRequest {
            jsonrpc: JSON_RPC_VERSION.to_string(),
            id: request_id.clone(),
            method: ADD_METHOD.to_string(),
            params: PluginAddParams { a, b },
        };
        let stdin = format!(
            "{}\n",
            serde_json::to_string(&rpc_request).map_err(|error| {
                PluginManagerError::InvalidResponse {
                    message: error.to_string(),
                }
            })?
        );
        let process_spec = self.process_spec();

        self.set_plugin_state(&plugin_id, PluginLifecycleState::Running);
        let process_output = self
            .spawn_and_run_plugin_process(&plugin_id, process_spec, stdin)
            .await;
        self.set_plugin_state(&plugin_id, PluginLifecycleState::Exited);
        let process_output = process_output?;

        parse_add_response(&plugin_id, &request_id, process_output)
    }

    /// Finds the plugin that is allowed to serve the requested method in this first slice.
    fn plugin_for_capability(
        &self,
        plugin_id: &str,
        method: &str,
    ) -> Result<&PluginDefinition, PluginManagerError> {
        let plugin =
            self.registry
                .get(plugin_id)
                .ok_or_else(|| PluginManagerError::PluginNotFound {
                    plugin_id: plugin_id.to_string(),
                })?;

        if plugin
            .capabilities
            .iter()
            .any(|capability| capability == method)
        {
            return Ok(plugin);
        }

        Err(PluginManagerError::CapabilityNotFound {
            plugin_id: plugin_id.to_string(),
            method: method.to_string(),
        })
    }

    /// Builds and drives one process crate invocation for the selected plugin.
    async fn spawn_and_run_plugin_process(
        &self,
        plugin_id: &str,
        process_spec: ProcessSpec,
        stdin: String,
    ) -> Result<PluginProcessOutput, PluginManagerError> {
        let mut process = self.process_spawner.spawn(process_spec).map_err(|error| {
            PluginManagerError::ProcessFailed {
                plugin_id: plugin_id.to_string(),
                message: format!("failed to spawn plugin process: {error}"),
            }
        })?;

        collect_plugin_process_output(plugin_id, &mut process, stdin, self.config.request_timeout)
            .await
    }

    /// Builds the process invocation from the manager data directory and bundled plugin layout.
    fn process_spec(&self) -> ProcessSpec {
        ProcessSpec::new(
            self.config
                .data_dir
                .join("bin")
                .join(bun_executable_name())
                .into_os_string(),
        )
        .arg(
            self.config
                .data_dir
                .join("plugins")
                .join("main.ts")
                .into_os_string(),
        )
        .cwd(self.config.data_dir.clone())
    }
}

impl<Spawner> PluginManager<Spawner> {
    /// Records the last known lifecycle state for one registered plugin.
    fn set_plugin_state(&self, plugin_id: &str, state: PluginLifecycleState) {
        self.lifecycle_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(plugin_id.to_string(), state);
    }
}

/// Describes one hardcoded plugin entry and its supported capabilities.
#[derive(Debug, PartialEq, Eq)]
struct PluginDefinition {
    id: String,
    capabilities: Vec<String>,
}

/// Parses stdout from the plugin process and returns the add result.
fn parse_add_response(
    plugin_id: &str,
    request_id: &str,
    output: PluginProcessOutput,
) -> Result<i64, PluginManagerError> {
    if output.exit_code != Some(0) {
        return Err(PluginManagerError::ProcessExitFailed {
            plugin_id: plugin_id.to_string(),
            exit_code: output.exit_code,
            stderr: output.stderr,
        });
    }

    let response_line =
        output
            .stdout
            .lines()
            .next()
            .ok_or_else(|| PluginManagerError::InvalidResponse {
                message: "plugin stdout did not contain a JSON-RPC response line".to_string(),
            })?;
    let response_value = serde_json::from_str::<serde_json::Value>(response_line)
        .map_err(|error| invalid_response_error(error.to_string()))?;

    if response_value.get("error").is_some() {
        let error_response = serde_json::from_value::<PluginJsonRpcErrorResponse>(response_value)
            .map_err(|error| invalid_response_error(error.to_string()))?;
        validate_response_version(&error_response.jsonrpc)?;
        validate_response_id(request_id, &error_response.id)?;

        return Err(PluginManagerError::JsonRpcError {
            code: error_response.error.code,
            message: error_response.error.message,
        });
    }

    let success_response = serde_json::from_value::<PluginJsonRpcSuccessResponse>(response_value)
        .map_err(|error| invalid_response_error(error.to_string()))?;
    validate_response_version(&success_response.jsonrpc)?;
    validate_response_id(request_id, &success_response.id)?;

    Ok(success_response.result)
}

/// Builds a stable invalid-response error from a parser failure message.
fn invalid_response_error(message: String) -> PluginManagerError {
    PluginManagerError::InvalidResponse { message }
}

/// Validates the JSON-RPC version returned by the plugin SDK.
fn validate_response_version(version: &str) -> Result<(), PluginManagerError> {
    if version == JSON_RPC_VERSION {
        return Ok(());
    }

    Err(PluginManagerError::InvalidResponse {
        message: format!("expected JSON-RPC version {JSON_RPC_VERSION}, got {version}"),
    })
}

/// Validates that the plugin response belongs to the request currently in flight.
fn validate_response_id(expected: &str, actual: &str) -> Result<(), PluginManagerError> {
    if actual == expected {
        return Ok(());
    }

    Err(PluginManagerError::ResponseIdMismatch {
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

/// Returns the bundled Bun executable name for the current platform.
fn bun_executable_name() -> &'static str {
    if cfg!(windows) { "bun.exe" } else { "bun" }
}

#[cfg(test)]
mod tests {
    use super::{ADD_PLUGIN_ID, PluginLifecycleState, PluginManager, bun_executable_name};
    use crate::{PluginManagerConfig, PluginManagerError};
    use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
    use pretty_assertions::assert_eq;
    use std::collections::VecDeque;
    use std::future;
    use std::io;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::process::ExitStatus;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::sync::Notify;

    /// Verifies add calls spawn the bundled plugin through the process crate boundary.
    #[tokio::test]
    async fn returns_add_result_and_drives_process_spec() {
        let spawner = FakeProcessSpawner::with_process(FakeProcessPlan::exited(
            "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"result\":3}\n",
            "",
            /*exit_code*/ 0,
        ));
        let data_dir = PathBuf::from("ora-data");
        let manager =
            PluginManager::new(PluginManagerConfig::new(data_dir.clone()), spawner.clone());

        let result = manager
            .number_add(1, 2)
            .await
            .unwrap_or_else(|error| panic!("expected add to succeed: {error}"));

        assert_eq!(result, 3);
        assert_eq!(
            manager.plugin_state(ADD_PLUGIN_ID),
            Some(PluginLifecycleState::Exited)
        );
        assert_eq!(
            spawner.specs(),
            vec![
                ProcessSpec::new(
                    data_dir
                        .join("bin")
                        .join(bun_executable_name())
                        .into_os_string(),
                )
                .arg(data_dir.join("plugins").join("main.ts").into_os_string())
                .cwd(data_dir)
            ],
        );
        assert_eq!(
            spawner.stdin_payloads(),
            vec![
                "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"add\",\"params\":{\"a\":1,\"b\":2}}\n"
                    .to_string()
            ],
        );
    }

    /// Verifies plugin lifecycle state moves through running while the process remains active.
    #[tokio::test]
    async fn exposes_running_state_during_plugin_invocation() {
        let spawner = FakeProcessSpawner::with_process(FakeProcessPlan::pending());
        let manager = Arc::new(PluginManager::new(
            PluginManagerConfig::with_request_timeout(
                PathBuf::from("ora-data"),
                Duration::from_millis(50),
            ),
            spawner.clone(),
        ));
        let task_manager = manager.clone();
        let add_task = tokio::spawn(async move { task_manager.number_add(1, 2).await });

        spawner.wait_for_spawn_count(1).await;

        assert_eq!(
            manager.plugin_state(ADD_PLUGIN_ID),
            Some(PluginLifecycleState::Running)
        );
        assert_eq!(
            add_task
                .await
                .unwrap_or_else(|error| panic!("expected task join to succeed: {error}")),
            Err(PluginManagerError::ProcessTimedOut {
                plugin_id: "1".to_string(),
            })
        );
        assert_eq!(
            manager.plugin_state(ADD_PLUGIN_ID),
            Some(PluginLifecycleState::Exited)
        );
        assert_eq!(spawner.kill_count(), 1);
    }

    /// Verifies malformed plugin stdout becomes a stable response error.
    #[tokio::test]
    async fn rejects_invalid_json_responses() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("ora-data")),
            FakeProcessSpawner::with_process(FakeProcessPlan::exited(
                "not-json\n",
                "",
                /*exit_code*/ 0,
            )),
        );

        let error = match manager.number_add(1, 2).await {
            Ok(result) => panic!("expected invalid JSON error, got result {result}"),
            Err(error) => error,
        };

        assert!(matches!(error, PluginManagerError::InvalidResponse { .. }));
    }

    /// Verifies response ids are checked before returning plugin results.
    #[tokio::test]
    async fn rejects_mismatched_response_ids() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("ora-data")),
            FakeProcessSpawner::with_process(FakeProcessPlan::exited(
                "{\"jsonrpc\":\"2.0\",\"id\":\"other\",\"result\":3}\n",
                "",
                /*exit_code*/ 0,
            )),
        );

        assert_eq!(
            manager.number_add(1, 2).await,
            Err(PluginManagerError::ResponseIdMismatch {
                expected: "1".to_string(),
                actual: "other".to_string(),
            })
        );
    }

    /// Verifies JSON-RPC errors from the plugin are preserved for callers.
    #[tokio::test]
    async fn maps_json_rpc_error_responses() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("ora-data")),
            FakeProcessSpawner::with_process(FakeProcessPlan::exited(
                "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"error\":{\"code\":-32601,\"message\":\"missing method\"}}\n",
                "",
                /*exit_code*/ 0,
            )),
        );

        assert_eq!(
            manager.number_add(1, 2).await,
            Err(PluginManagerError::JsonRpcError {
                code: -32601,
                message: "missing method".to_string(),
            })
        );
    }

    /// Verifies nonzero process exits are reported before stdout parsing.
    #[tokio::test]
    async fn maps_nonzero_process_exits() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("ora-data")),
            FakeProcessSpawner::with_process(FakeProcessPlan::exited(
                "", "boom", /*exit_code*/ 1,
            )),
        );

        assert_eq!(
            manager.number_add(1, 2).await,
            Err(PluginManagerError::ProcessExitFailed {
                plugin_id: "1".to_string(),
                exit_code: Some(1),
                stderr: "boom".to_string(),
            })
        );
    }

    /// Verifies process spawn failures become stable plugin-manager errors.
    #[tokio::test]
    async fn maps_spawn_failures() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("ora-data")),
            FakeProcessSpawner::with_spawn_error("missing bun"),
        );

        assert_eq!(
            manager.number_add(1, 2).await,
            Err(PluginManagerError::ProcessFailed {
                plugin_id: "1".to_string(),
                message: "failed to spawn plugin process: missing bun".to_string(),
            })
        );
        assert_eq!(
            manager.plugin_state(ADD_PLUGIN_ID),
            Some(PluginLifecycleState::Exited)
        );
    }

    /// Verifies a missing process pipe is reported before protocol parsing starts.
    #[tokio::test]
    async fn maps_missing_stdout_pipe() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("ora-data")),
            FakeProcessSpawner::with_process(
                FakeProcessPlan::exited(
                    "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"result\":3}\n",
                    "",
                    /*exit_code*/ 0,
                )
                .without_stdout(),
            ),
        );

        assert_eq!(
            manager.number_add(1, 2).await,
            Err(PluginManagerError::ProcessFailed {
                plugin_id: "1".to_string(),
                message: "process did not expose stdout pipe".to_string(),
            })
        );
    }

    #[derive(Clone)]
    struct FakeProcessSpawner {
        inner: Arc<FakeProcessSpawnerState>,
    }

    struct FakeProcessSpawnerState {
        plans: Mutex<VecDeque<FakeSpawnPlan>>,
        specs: Mutex<Vec<ProcessSpec>>,
        stdin_payloads: Mutex<Vec<Arc<Mutex<Vec<u8>>>>>,
        kill_count: Arc<AtomicUsize>,
        spawn_notify: Notify,
    }

    impl FakeProcessSpawner {
        /// Builds a fake process spawner that returns one planned process handle.
        fn with_process(plan: FakeProcessPlan) -> Self {
            Self::with_plans([FakeSpawnPlan::Process(plan)])
        }

        /// Builds a fake process spawner that fails before returning a process handle.
        fn with_spawn_error(message: impl Into<String>) -> Self {
            Self::with_plans([FakeSpawnPlan::SpawnError(message.into())])
        }

        /// Builds a fake process spawner with the requested spawn sequence.
        fn with_plans(plans: impl IntoIterator<Item = FakeSpawnPlan>) -> Self {
            Self {
                inner: Arc::new(FakeProcessSpawnerState {
                    plans: Mutex::new(plans.into_iter().collect()),
                    specs: Mutex::new(Vec::new()),
                    stdin_payloads: Mutex::new(Vec::new()),
                    kill_count: Arc::new(AtomicUsize::new(0)),
                    spawn_notify: Notify::new(),
                }),
            }
        }

        /// Returns every process spec received by the fake process boundary.
        fn specs(&self) -> Vec<ProcessSpec> {
            self.inner
                .specs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// Returns every stdin payload written by the manager.
        fn stdin_payloads(&self) -> Vec<String> {
            self.inner
                .stdin_payloads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|payload| {
                    let bytes = payload
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    String::from_utf8(bytes)
                        .unwrap_or_else(|error| panic!("expected utf8 stdin payload: {error}"))
                })
                .collect()
        }

        /// Returns how many times fake process handles were killed.
        fn kill_count(&self) -> usize {
            self.inner.kill_count.load(Ordering::SeqCst)
        }

        /// Waits until the manager has crossed the process spawn boundary.
        async fn wait_for_spawn_count(&self, expected_count: usize) {
            let wait_result = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if self.specs().len() >= expected_count {
                        return;
                    }
                    self.inner.spawn_notify.notified().await;
                }
            })
            .await;

            wait_result.unwrap_or_else(|_| {
                panic!("expected at least {expected_count} fake process spawn calls")
            });
        }
    }

    impl ProcessSpawner for FakeProcessSpawner {
        type Process = FakeProcess;

        fn spawn(&self, spec: ProcessSpec) -> io::Result<Self::Process> {
            self.inner
                .specs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(spec);
            self.inner.spawn_notify.notify_waiters();

            let plan = self
                .inner
                .plans
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| panic!("missing fake process plan"));

            match plan {
                FakeSpawnPlan::Process(plan) => {
                    let stdin_payload = Arc::new(Mutex::new(Vec::new()));
                    self.inner
                        .stdin_payloads
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(stdin_payload.clone());

                    Ok(FakeProcess {
                        stdin: plan.pipes.stdin.then_some(RecordingStdin { stdin_payload }),
                        stdout: plan
                            .pipes
                            .stdout
                            .then(|| FakeRead::new(plan.stdout.into_bytes())),
                        stderr: plan
                            .pipes
                            .stderr
                            .then(|| FakeRead::new(plan.stderr.into_bytes())),
                        exit: plan.exit,
                        kill_count: self.inner.kill_count.clone(),
                    })
                }
                FakeSpawnPlan::SpawnError(message) => Err(io::Error::other(message)),
            }
        }
    }

    enum FakeSpawnPlan {
        Process(FakeProcessPlan),
        SpawnError(String),
    }

    struct FakeProcessPlan {
        stdout: String,
        stderr: String,
        exit: FakeExit,
        pipes: FakePipes,
    }

    impl FakeProcessPlan {
        /// Builds a fake process that exits with configured output.
        fn exited(stdout: impl Into<String>, stderr: impl Into<String>, exit_code: i32) -> Self {
            Self {
                stdout: stdout.into(),
                stderr: stderr.into(),
                exit: FakeExit::Exited(exit_code),
                pipes: FakePipes::all(),
            }
        }

        /// Builds a fake process that never exits until the manager timeout path kills it.
        fn pending() -> Self {
            Self {
                stdout: String::new(),
                stderr: String::new(),
                exit: FakeExit::Pending,
                pipes: FakePipes::all(),
            }
        }

        /// Removes stdout to exercise process boundary validation.
        fn without_stdout(mut self) -> Self {
            self.pipes.stdout = false;
            self
        }
    }

    struct FakePipes {
        stdin: bool,
        stdout: bool,
        stderr: bool,
    }

    impl FakePipes {
        /// Enables all three stdio pipes expected by plugin-manager.
        fn all() -> Self {
            Self {
                stdin: true,
                stdout: true,
                stderr: true,
            }
        }
    }

    struct FakeProcess {
        stdin: Option<RecordingStdin>,
        stdout: Option<FakeRead>,
        stderr: Option<FakeRead>,
        exit: FakeExit,
        kill_count: Arc<AtomicUsize>,
    }

    impl ManagedProcess for FakeProcess {
        type Stdin = RecordingStdin;
        type Stdout = FakeRead;
        type Stderr = FakeRead;

        fn id(&self) -> Option<u32> {
            Some(42)
        }

        fn take_stdin(&mut self) -> Option<Self::Stdin> {
            self.stdin.take()
        }

        fn take_stdout(&mut self) -> Option<Self::Stdout> {
            self.stdout.take()
        }

        fn take_stderr(&mut self) -> Option<Self::Stderr> {
            self.stderr.take()
        }

        fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
            match self.exit {
                FakeExit::Exited(exit_code) => Ok(Some(exit_status(exit_code))),
                FakeExit::Pending => Ok(None),
            }
        }

        async fn wait(&self) -> io::Result<ExitStatus> {
            match self.exit {
                FakeExit::Exited(exit_code) => Ok(exit_status(exit_code)),
                FakeExit::Pending => future::pending().await,
            }
        }

        async fn kill(&self) -> io::Result<()> {
            self.kill_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Clone, Copy)]
    enum FakeExit {
        Exited(i32),
        Pending,
    }

    struct FakeRead {
        cursor: std::io::Cursor<Vec<u8>>,
    }

    impl FakeRead {
        /// Builds an async reader over deterministic bytes.
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                cursor: std::io::Cursor::new(bytes),
            }
        }
    }

    impl AsyncRead for FakeRead {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let position = self.cursor.position() as usize;
            let source = self.cursor.get_ref();
            if position >= source.len() || buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }

            let read_length = (source.len() - position).min(buf.remaining());
            buf.put_slice(&source[position..position + read_length]);
            self.cursor.set_position((position + read_length) as u64);

            Poll::Ready(Ok(()))
        }
    }

    struct RecordingStdin {
        stdin_payload: Arc<Mutex<Vec<u8>>>,
    }

    impl AsyncWrite for RecordingStdin {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.stdin_payload
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(buf);

            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[cfg(windows)]
    fn exit_status(exit_code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(exit_code as u32)
    }

    #[cfg(unix)]
    fn exit_status(exit_code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(exit_code << 8)
    }
}
