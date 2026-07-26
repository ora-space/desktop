use super::{resolve_agent_cli_path, support::contract_agent_cli};
use ora_contracts::{AgentCliModels, ListAgentModelsResponse};
use ora_domain::AgentCli;
use ora_logging::ora_warn;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Queries every supported CLI concurrently and omits providers that cannot report models.
pub(super) async fn list_agent_models(home_directory: &Path) -> ListAgentModelsResponse {
    let (opencode, nga, code_agent_cli) = tokio::join!(
        query_agent_models(AgentCli::OpenCode, home_directory),
        query_agent_models(AgentCli::Nga, home_directory),
        query_agent_models(AgentCli::CodeAgentCli, home_directory),
    );
    ListAgentModelsResponse {
        groups: [opencode, nga, code_agent_cli]
            .into_iter()
            .flatten()
            .collect(),
    }
}

/// Runs a bounded, short-lived discovery command so a broken CLI cannot block the aggregate API.
async fn query_agent_models(agent_cli: AgentCli, home_directory: &Path) -> Option<AgentCliModels> {
    let executable = resolve_agent_cli_path(agent_cli, home_directory).ok()?;
    let mut command = Command::new(executable);
    command
        .arg("models")
        .current_dir(home_directory)
        .kill_on_drop(true);
    let output = match timeout(MODEL_LIST_TIMEOUT, command.output()).await {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            ora_warn!(
                agent_cli = agent_cli.database_value(),
                status = %output.status,
                "agent CLI model discovery failed"
            );
            return None;
        }
        Ok(Err(error)) => {
            ora_warn!(
                agent_cli = agent_cli.database_value(),
                error = %error,
                "agent CLI model discovery could not start"
            );
            return None;
        }
        Err(_) => {
            ora_warn!(
                agent_cli = agent_cli.database_value(),
                "agent CLI model discovery timed out"
            );
            return None;
        }
    };
    let models = parse_models(&output.stdout)?;
    Some(AgentCliModels {
        agent_cli: contract_agent_cli(agent_cli),
        models,
    })
}

/// Normalizes line-oriented CLI output into deterministic model identifiers.
fn parse_models(stdout: &[u8]) -> Option<Vec<String>> {
    let stdout = std::str::from_utf8(stdout).ok()?;
    let mut models = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    models.sort_unstable();
    models.dedup();
    Some(models)
}

#[cfg(test)]
mod tests {
    use super::parse_models;
    use pretty_assertions::assert_eq;

    /// Verifies discovery output is trimmed, stable, and free of duplicate identifiers.
    #[test]
    fn parses_model_output() {
        assert_eq!(
            parse_models(b" provider/zeta\nprovider/alpha\n\nprovider/zeta\n"),
            Some(vec![
                "provider/alpha".to_string(),
                "provider/zeta".to_string(),
            ])
        );
    }

    /// Verifies invalid process output cannot leak lossy identifiers into the public contract.
    #[test]
    fn rejects_non_utf8_model_output() {
        assert_eq!(parse_models(&[0xff]), None);
    }

    /// Verifies unavailable CLIs are omitted while a working discovery command remains visible.
    #[cfg(unix)]
    #[tokio::test]
    async fn lists_only_successful_cli_groups() {
        use super::list_agent_models;
        use ora_contracts::{AgentCli, AgentCliModels, ListAgentModelsResponse};
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::TempDir::new().unwrap();
        let bin_directory = home.path().join(".opencode").join("bin");
        fs::create_dir_all(&bin_directory).unwrap();
        let executable = bin_directory.join("opencode");
        fs::write(
            &executable,
            "#!/bin/sh\nprintf 'provider/zeta\\nprovider/alpha\\nprovider/zeta\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            list_agent_models(home.path()).await,
            ListAgentModelsResponse {
                groups: vec![AgentCliModels {
                    agent_cli: AgentCli::OpenCode,
                    models: vec!["provider/alpha".to_string(), "provider/zeta".to_string(),],
                }],
            }
        );
    }
}
