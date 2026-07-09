use crate::config::PluginManagerConfig;
use crate::error::PluginManagerError;
use crate::process::{PluginProcessOutput, PluginProcessRequest, PluginProcessRuntime};
use ora_contracts::{
    PluginAddParams, PluginJsonRpcErrorResponse, PluginJsonRpcRequest, PluginJsonRpcSuccessResponse,
};
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
pub struct PluginManager<Runtime> {
    config: PluginManagerConfig,
    runtime: Runtime,
    registry: HashMap<String, PluginDefinition>,
    lifecycle_states: Mutex<HashMap<String, PluginLifecycleState>>,
}

impl<Runtime> PluginManager<Runtime> {
    /// Builds the plugin manager around a process runtime implementation.
    pub fn new(config: PluginManagerConfig, runtime: Runtime) -> Self {
        let registry = HashMap::from([(ADD_PLUGIN_ID.to_string(), PluginDefinition::add_plugin())]);
        let lifecycle_states = Mutex::new(HashMap::from([(
            ADD_PLUGIN_ID.to_string(),
            PluginLifecycleState::Registered,
        )]));

        Self {
            config,
            runtime,
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

impl<Runtime> PluginManager<Runtime>
where
    Runtime: PluginProcessRuntime,
{
    /// Calls the hardcoded add capability through the configured plugin process runtime.
    pub fn number_add(&self, a: i64, b: i64) -> Result<i64, PluginManagerError> {
        let plugin = self.plugin_for_capability(ADD_PLUGIN_ID, ADD_METHOD)?;
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
        let process_request = self.process_request(plugin, stdin);

        self.set_plugin_state(&plugin.id, PluginLifecycleState::Running);
        let process_output = self.runtime.run_plugin_process(process_request);
        self.set_plugin_state(&plugin.id, PluginLifecycleState::Exited);
        let process_output = process_output
            .map_err(|error| PluginManagerError::from_process_runtime_error(&plugin.id, error))?;

        parse_add_response(&plugin.id, &request_id, process_output)
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

    /// Builds the process invocation from plugin metadata and the manager data directory.
    fn process_request(&self, plugin: &PluginDefinition, stdin: String) -> PluginProcessRequest {
        PluginProcessRequest {
            plugin_id: plugin.id.clone(),
            program: self.config.data_dir.join("bin").join(bun_executable_name()),
            args: vec![self.config.data_dir.join("plugins").join("main.ts")],
            cwd: self.config.data_dir.clone(),
            stdin,
            timeout: self.config.request_timeout,
        }
    }
}

impl<Runtime> PluginManager<Runtime> {
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

impl PluginDefinition {
    /// Builds the first-slice add plugin definition.
    fn add_plugin() -> Self {
        Self {
            id: ADD_PLUGIN_ID.to_string(),
            capabilities: vec![ADD_METHOD.to_string()],
        }
    }
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
    use crate::{
        PluginManagerConfig, PluginManagerError, PluginProcessOutput, PluginProcessRequest,
        PluginProcessRuntime, PluginProcessRuntimeError,
    };
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    /// Verifies add calls return the JSON-RPC result produced by the plugin process.
    #[test]
    fn returns_add_result_from_plugin_response() {
        let runtime = Rc::new(FakePluginProcessRuntime::with_output(PluginProcessOutput {
            stdout: "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"result\":3}\n".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        }));
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("/tmp/ora-data")),
            runtime.clone(),
        );

        let result = manager
            .number_add(1, 2)
            .unwrap_or_else(|error| panic!("expected add to succeed: {error}"));

        assert_eq!(result, 3);
        assert_eq!(
            manager.plugin_state(ADD_PLUGIN_ID),
            Some(PluginLifecycleState::Exited)
        );
        assert_eq!(
            runtime.requests(),
            vec![PluginProcessRequest {
                plugin_id: "1".to_string(),
                program: PathBuf::from("/tmp/ora-data")
                    .join("bin")
                    .join(bun_executable_name()),
                args: vec![PathBuf::from("/tmp/ora-data")
                    .join("plugins")
                    .join("main.ts")],
                cwd: PathBuf::from("/tmp/ora-data"),
                stdin: "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"add\",\"params\":{\"a\":1,\"b\":2}}\n"
                    .to_string(),
                timeout: Duration::from_secs(5),
            }]
        );
    }

    /// Verifies plugin lifecycle state moves through running before the process returns.
    #[test]
    fn exposes_running_state_during_plugin_invocation() {
        let runtime = Rc::new(FakePluginProcessRuntime::with_output(PluginProcessOutput {
            stdout: "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"result\":3}\n".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        }));
        let manager = Rc::new(PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("/tmp/ora-data")),
            runtime.clone(),
        ));
        runtime.observe_state_during_run(manager.clone());

        let result = manager
            .number_add(1, 2)
            .unwrap_or_else(|error| panic!("expected add to succeed: {error}"));

        assert_eq!(result, 3);
        assert_eq!(
            runtime.observed_states(),
            vec![Some(PluginLifecycleState::Running)]
        );
        assert_eq!(
            manager.plugin_state(ADD_PLUGIN_ID),
            Some(PluginLifecycleState::Exited)
        );
    }

    /// Verifies malformed plugin stdout becomes a stable response error.
    #[test]
    fn rejects_invalid_json_responses() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("/tmp/ora-data")),
            Rc::new(FakePluginProcessRuntime::with_output(PluginProcessOutput {
                stdout: "not-json\n".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
            })),
        );

        let error = manager.number_add(1, 2).unwrap_err();

        assert!(matches!(error, PluginManagerError::InvalidResponse { .. }));
    }

    /// Verifies response ids are checked before returning plugin results.
    #[test]
    fn rejects_mismatched_response_ids() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("/tmp/ora-data")),
            Rc::new(FakePluginProcessRuntime::with_output(PluginProcessOutput {
                stdout: "{\"jsonrpc\":\"2.0\",\"id\":\"other\",\"result\":3}\n".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
            })),
        );

        assert_eq!(
            manager.number_add(1, 2),
            Err(PluginManagerError::ResponseIdMismatch {
                expected: "1".to_string(),
                actual: "other".to_string(),
            })
        );
    }

    /// Verifies JSON-RPC errors from the plugin are preserved for callers.
    #[test]
    fn maps_json_rpc_error_responses() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("/tmp/ora-data")),
            Rc::new(FakePluginProcessRuntime::with_output(PluginProcessOutput {
                stdout: "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"error\":{\"code\":-32601,\"message\":\"missing method\"}}\n"
                    .to_string(),
                stderr: String::new(),
                exit_code: Some(0),
            })),
        );

        assert_eq!(
            manager.number_add(1, 2),
            Err(PluginManagerError::JsonRpcError {
                code: -32601,
                message: "missing method".to_string(),
            })
        );
    }

    /// Verifies nonzero process exits are reported before stdout parsing.
    #[test]
    fn maps_nonzero_process_exits() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("/tmp/ora-data")),
            Rc::new(FakePluginProcessRuntime::with_output(PluginProcessOutput {
                stdout: String::new(),
                stderr: "boom".to_string(),
                exit_code: Some(1),
            })),
        );

        assert_eq!(
            manager.number_add(1, 2),
            Err(PluginManagerError::ProcessExitFailed {
                plugin_id: "1".to_string(),
                exit_code: Some(1),
                stderr: "boom".to_string(),
            })
        );
    }

    /// Verifies process timeout failures become a stable plugin-manager error.
    #[test]
    fn maps_process_timeouts() {
        let manager = PluginManager::new(
            PluginManagerConfig::new(PathBuf::from("/tmp/ora-data")),
            Rc::new(FakePluginProcessRuntime::with_error(
                PluginProcessRuntimeError::TimedOut,
            )),
        );

        assert_eq!(
            manager.number_add(1, 2),
            Err(PluginManagerError::ProcessTimedOut {
                plugin_id: "1".to_string(),
            })
        );
        assert_eq!(
            manager.plugin_state(ADD_PLUGIN_ID),
            Some(PluginLifecycleState::Exited)
        );
    }

    struct FakePluginProcessRuntime {
        output: RefCell<Result<PluginProcessOutput, PluginProcessRuntimeError>>,
        requests: RefCell<Vec<PluginProcessRequest>>,
        state_observer: RefCell<Option<Rc<PluginManager<Rc<FakePluginProcessRuntime>>>>>,
        observed_states: RefCell<Vec<Option<PluginLifecycleState>>>,
    }

    impl FakePluginProcessRuntime {
        /// Builds a fake runtime that returns one deterministic process output.
        fn with_output(output: PluginProcessOutput) -> Self {
            Self {
                output: RefCell::new(Ok(output)),
                requests: RefCell::new(Vec::new()),
                state_observer: RefCell::new(None),
                observed_states: RefCell::new(Vec::new()),
            }
        }

        /// Builds a fake runtime that returns one deterministic process error.
        fn with_error(error: PluginProcessRuntimeError) -> Self {
            Self {
                output: RefCell::new(Err(error)),
                requests: RefCell::new(Vec::new()),
                state_observer: RefCell::new(None),
                observed_states: RefCell::new(Vec::new()),
            }
        }

        /// Records manager state while the fake process invocation is in flight.
        fn observe_state_during_run(
            &self,
            manager: Rc<PluginManager<Rc<FakePluginProcessRuntime>>>,
        ) {
            self.state_observer.replace(Some(manager));
        }

        /// Returns every captured process request.
        fn requests(&self) -> Vec<PluginProcessRequest> {
            self.requests.borrow().clone()
        }

        /// Returns every lifecycle state observed from inside the fake runtime.
        fn observed_states(&self) -> Vec<Option<PluginLifecycleState>> {
            self.observed_states.borrow().clone()
        }
    }

    impl PluginProcessRuntime for Rc<FakePluginProcessRuntime> {
        fn run_plugin_process(
            &self,
            request: PluginProcessRequest,
        ) -> Result<PluginProcessOutput, PluginProcessRuntimeError> {
            self.requests.borrow_mut().push(request);
            if let Some(manager) = self.state_observer.borrow().as_ref() {
                self.observed_states
                    .borrow_mut()
                    .push(manager.plugin_state(ADD_PLUGIN_ID));
            }

            self.output.borrow().clone()
        }
    }
}
