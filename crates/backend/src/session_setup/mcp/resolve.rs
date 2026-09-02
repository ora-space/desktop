//! Pure Session MCP resolver: Effective MCP Set → ACP `mcpServers` + secret-free revision.

use super::{
    AgentSessionMcpCapabilities, InstalledMcpCandidate, McpConfigurationEligibility,
    SessionMcpCatalog, SessionMcpConfigurationSource, SessionMcpError, SessionMcpMemberRevision,
    SessionMcpRevision, SessionMcpSnapshot, SessionMcpTransportKind,
};
use agent_client_protocol_schema::v1::{
    EnvVariable, HttpHeader, McpServer, McpServerHttp, McpServerStdio,
};
use ora_plugin_config::{
    ResolveMcpBindingError, ResolvedMcpArgument, ResolvedMcpTransport, resolve_mcp_transport,
};
use ora_utils::path::CanonicalPathRoot;
use std::path::Path;

const MAX_REVISION_RETRIES: usize = 3;

/// Builds the secret-free Desired revision without producing ACP payloads.
///
/// Prompt admission and wakeup paths use this so Setting values never enter digest comparison.
pub(crate) fn resolve_session_mcp_revision(
    catalog: &impl SessionMcpCatalog,
    configurations: &impl SessionMcpConfigurationSource,
) -> Result<SessionMcpRevision, SessionMcpError> {
    let selected = select_effective_set(catalog, configurations)?;
    Ok(revision_from_selected(&selected))
}

/// Resolves one complete, ordered Session MCP Snapshot for a single ACP setup or refresh.
///
/// The function is side-effect free. If any selected member's package version or configuration
/// revision changes while it runs, it regenerates the whole Snapshot rather than splicing
/// revisions. A remaining mismatch fails closed so a partial `mcpServers` list is never returned.
pub(crate) fn resolve_session_mcp(
    catalog: &impl SessionMcpCatalog,
    configurations: &impl SessionMcpConfigurationSource,
    cwd: &Path,
    capabilities: AgentSessionMcpCapabilities,
) -> Result<SessionMcpSnapshot, SessionMcpError> {
    for _ in 0..MAX_REVISION_RETRIES {
        let selected = select_effective_set(catalog, configurations)?;
        let snapshot = build_snapshot(&selected, cwd, capabilities)?;
        let current = select_effective_set(catalog, configurations)?;
        if identities_match(&selected, &current) {
            return Ok(snapshot);
        }
    }
    Err(SessionMcpError::RevisionChanged { plugin_id: None })
}

struct SelectedMcp {
    candidate: InstalledMcpCandidate,
    configuration_revision: u64,
    values: std::collections::BTreeMap<String, ora_plugin_config::SettingValue>,
}

/// Enumerates installed MCP plugins and keeps only those whose configuration is currently complete.
fn select_effective_set(
    catalog: &impl SessionMcpCatalog,
    configurations: &impl SessionMcpConfigurationSource,
) -> Result<Vec<SelectedMcp>, SessionMcpError> {
    let mut candidates = catalog
        .installed_mcps()
        .map_err(|_| SessionMcpError::CatalogUnavailable)?;
    candidates.sort_by_key(|candidate| candidate.plugin_id.canonical());
    let mut selected = Vec::new();
    for candidate in candidates {
        match configurations.eligibility(&candidate)? {
            McpConfigurationEligibility::Incomplete => {}
            McpConfigurationEligibility::NoSettings => selected.push(SelectedMcp {
                candidate,
                configuration_revision: 0,
                values: std::collections::BTreeMap::new(),
            }),
            McpConfigurationEligibility::Complete { revision, values } => {
                selected.push(SelectedMcp {
                    candidate,
                    configuration_revision: revision,
                    values,
                });
            }
            McpConfigurationEligibility::Unavailable => {
                return Err(SessionMcpError::ConfigurationUnavailable {
                    plugin_id: candidate.plugin_id,
                });
            }
        }
    }
    Ok(selected)
}

fn identities_match(left: &[SelectedMcp], right: &[SelectedMcp]) -> bool {
    revision_from_selected(left) == revision_from_selected(right)
}

fn revision_from_selected(selected: &[SelectedMcp]) -> SessionMcpRevision {
    SessionMcpRevision::new(
        selected
            .iter()
            .map(|member| SessionMcpMemberRevision {
                plugin_id: member.candidate.plugin_id.clone(),
                package_version: member.candidate.version.clone(),
                configuration_revision: member.configuration_revision,
                transport: match member.candidate.configuration.transport {
                    ora_plugin_config::McpTransport::Stdio(_) => SessionMcpTransportKind::Stdio,
                    ora_plugin_config::McpTransport::Http(_) => SessionMcpTransportKind::Http,
                },
            })
            .collect(),
    )
}

/// Maps every selected member into ACP servers, failing the whole Snapshot if one member cannot.
fn build_snapshot(
    selected: &[SelectedMcp],
    cwd: &Path,
    capabilities: AgentSessionMcpCapabilities,
) -> Result<SessionMcpSnapshot, SessionMcpError> {
    if !selected.is_empty() && !capabilities.load_session {
        return Err(SessionMcpError::LoadCapabilityMissing);
    }
    let mut servers = Vec::with_capacity(selected.len());
    for member in selected {
        servers.push(map_member(member, cwd, capabilities)?);
    }
    Ok(SessionMcpSnapshot::new(
        servers,
        revision_from_selected(selected),
    ))
}

fn map_member(
    member: &SelectedMcp,
    cwd: &Path,
    capabilities: AgentSessionMcpCapabilities,
) -> Result<McpServer, SessionMcpError> {
    let transport = match member.candidate.configuration.transport {
        ora_plugin_config::McpTransport::Stdio(_) => SessionMcpTransportKind::Stdio,
        ora_plugin_config::McpTransport::Http(_) => SessionMcpTransportKind::Http,
    };
    if transport == SessionMcpTransportKind::Http && !capabilities.http {
        return Err(SessionMcpError::HttpCapabilityMissing {
            plugin_id: member.candidate.plugin_id.clone(),
        });
    }
    let resolved = resolve_mcp_transport(&member.candidate.configuration, &member.values).map_err(
        |error| match error {
            ResolveMcpBindingError::MissingSetting { setting_id } => {
                SessionMcpError::SettingMissing {
                    plugin_id: member.candidate.plugin_id.clone(),
                    setting_id,
                    transport,
                }
            }
            ResolveMcpBindingError::IllegalRuntimeText { setting_id } => {
                SessionMcpError::IllegalRuntimeText {
                    plugin_id: member.candidate.plugin_id.clone(),
                    setting_id,
                    transport,
                }
            }
        },
    )?;
    let name = member.candidate.plugin_id.canonical();
    match resolved {
        ResolvedMcpTransport::Stdio { command, args, env } => {
            let command_path = revalidate_stdio_command(
                &member.candidate.package_root,
                &command,
                &member.candidate.plugin_id,
            )?;
            let mut mapped_args = Vec::with_capacity(args.len());
            for argument in args {
                mapped_args.push(match argument {
                    ResolvedMcpArgument::Literal(value) => value,
                    ResolvedMcpArgument::WorkspaceContext => {
                        if !cwd.is_absolute() {
                            return Err(SessionMcpError::WorkspaceCwdUnresolved {
                                plugin_id: member.candidate.plugin_id.clone(),
                            });
                        }
                        cwd.to_string_lossy().into_owned()
                    }
                });
            }
            Ok(McpServer::Stdio(
                McpServerStdio::new(name, command_path)
                    .args(mapped_args)
                    .env(
                        env.into_iter()
                            .map(|(env_name, value)| EnvVariable::new(env_name, value))
                            .collect(),
                    ),
            ))
        }
        ResolvedMcpTransport::Http { url, headers } => Ok(McpServer::Http(
            McpServerHttp::new(name, url.as_str()).headers(
                headers
                    .into_iter()
                    .map(|(header_name, value)| HttpHeader::new(header_name, value))
                    .collect(),
            ),
        )),
    }
}

/// Re-checks that the stdio command is still an ordinary file inside this exact package version.
fn revalidate_stdio_command(
    package_root: &Path,
    command: &ora_utils::path::PortableRelativePath,
    plugin_id: &ora_domain::PluginId,
) -> Result<std::path::PathBuf, SessionMcpError> {
    let not_in_package = || SessionMcpError::CommandNotInPackage {
        plugin_id: plugin_id.clone(),
    };
    let root = CanonicalPathRoot::new(package_root).map_err(|_| not_in_package())?;
    let resolved = root
        .resolve_existing(command)
        .map_err(|_| not_in_package())?;
    if !resolved.is_file() {
        return Err(not_in_package());
    }
    root.relative_path(&resolved)
        .map_err(|_| not_in_package())?;
    Ok(resolved)
}
