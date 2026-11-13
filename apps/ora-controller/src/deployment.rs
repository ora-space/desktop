use super::*;
use serde::{Deserialize, Serialize};

/// Deployment state for the executable: the shared runtime configuration and the optional Node
/// this process may host. Hosting is chosen on the command line.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentConfig {
    pub controller: RuntimeConfig,
    pub single_node: Option<SingleNodeConfig>,
}

/// How to start the one configured Node when `--single-node` is given.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SingleNodeConfig {
    pub node_executable: PathBuf,
    pub node_config: PathBuf,
    pub ready_timeout_ms: u64,
    pub stop_timeout_ms: u64,
}

/// Whether this process starts the configured Node or only connects to an externally managed one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeHosting {
    Managed,
    External,
}

impl DeploymentConfig {
    /// Rejects an incomplete hosting request before any state is opened.
    pub fn validate(&self, hosting: NodeHosting) -> Result<Option<&SingleNodeConfig>, Error> {
        match hosting {
            NodeHosting::External => Ok(None),
            NodeHosting::Managed => {
                let single = self.single_node.as_ref().ok_or_else(|| {
                    Error::Configuration("--single-node requires a single_node section".into())
                })?;
                // Hosting is only defined for a deployment whose one static Node it can start.
                if self.controller.nodes.len() != 1
                    || single.ready_timeout_ms == 0
                    || single.stop_timeout_ms == 0
                    || !single.node_executable.is_absolute()
                    || !single.node_config.is_absolute()
                {
                    return Err(Error::Configuration(
                        "single_node requires exactly one configured Node, absolute paths and nonzero timeouts".into(),
                    ));
                }
                Ok(Some(single))
            }
        }
    }
}
