//! One configuration-complete installed MCP member, shared by delivery and health consumers.
//!
//! Both the Session MCP resolver and the Host health probe enumerate through
//! [`effective_members`], so membership, ordering, and explicit-selection failure semantics can
//! never drift between what is delivered and what is probed.

use super::{
    InstalledMcpCandidate, McpConfigurationEligibility, SessionMcpCatalog,
    SessionMcpConfigurationSource, SessionMcpError, SessionMcpSelection, SessionMcpTransportKind,
};
use ora_plugin_config::SettingValue;
use std::collections::BTreeMap;

/// One installed MCP whose current configuration is complete enough to be bound.
#[derive(Clone, Debug)]
pub(crate) struct EligibleMcpMember {
    pub candidate: InstalledMcpCandidate,
    pub configuration_revision: u64,
    pub values: BTreeMap<String, SettingValue>,
}

impl EligibleMcpMember {
    /// Secret-free transport kind for identity construction.
    pub(crate) fn transport_kind(&self) -> SessionMcpTransportKind {
        match self.candidate.configuration.transport {
            ora_plugin_config::McpTransport::Stdio(_) => SessionMcpTransportKind::Stdio,
            ora_plugin_config::McpTransport::Http(_) => SessionMcpTransportKind::Http,
        }
    }

    /// Whether the compiled transport substitutes the Session cwd into its argv.
    ///
    /// Such a member can only be probed with a real absolute Session cwd: a probe without one
    /// would have to invent a placeholder, which the health identity forbids.
    pub(crate) fn needs_workspace_context(&self) -> bool {
        match &self.candidate.configuration.transport {
            ora_plugin_config::McpTransport::Stdio(transport) => {
                transport.args.iter().any(|argument| {
                    matches!(argument, ora_plugin_config::McpArgument::WorkspaceContext)
                })
            }
            ora_plugin_config::McpTransport::Http(_) => false,
        }
    }
}

/// Enumerates the Effective MCP Set for one Session selection.
///
/// Automatic selection keeps every currently eligible installed MCP in canonical Plugin ID
/// order. Explicit selection keeps only authorized members and fails closed when a member the
/// author named is missing or still incomplete, so author intent cannot silently shrink. An
/// explicit empty set resolves to no members and never falls back to automatic discovery.
pub(crate) fn effective_members(
    catalog: &impl SessionMcpCatalog,
    configurations: &impl SessionMcpConfigurationSource,
    selection: &SessionMcpSelection,
) -> Result<Vec<EligibleMcpMember>, SessionMcpError> {
    // An explicit empty set must work even when unrelated installed plugins are broken.
    if matches!(selection, SessionMcpSelection::Explicit(ids) if ids.is_empty()) {
        return Ok(Vec::new());
    }
    let mut candidates = catalog
        .installed_mcps()
        .map_err(|_| SessionMcpError::CatalogUnavailable)?;
    if let SessionMcpSelection::Explicit(ids) = selection {
        for id in ids {
            if !candidates
                .iter()
                .any(|candidate| candidate.plugin_id == *id)
            {
                return Err(SessionMcpError::SelectedPluginUnavailable {
                    plugin_id: id.clone(),
                });
            }
        }
        candidates.retain(|candidate| ids.contains(&candidate.plugin_id));
    }
    candidates.sort_by_key(|candidate| candidate.plugin_id.canonical());
    let mut selected = Vec::new();
    for candidate in candidates {
        match configurations.eligibility(&candidate)? {
            McpConfigurationEligibility::Incomplete => {
                if matches!(selection, SessionMcpSelection::Explicit(_)) {
                    return Err(SessionMcpError::ConfigurationIncomplete {
                        plugin_id: candidate.plugin_id,
                        transport: match candidate.configuration.transport {
                            ora_plugin_config::McpTransport::Stdio(_) => {
                                SessionMcpTransportKind::Stdio
                            }
                            ora_plugin_config::McpTransport::Http(_) => {
                                SessionMcpTransportKind::Http
                            }
                        },
                    });
                }
            }
            McpConfigurationEligibility::NoSettings => selected.push(EligibleMcpMember {
                candidate,
                configuration_revision: 0,
                values: BTreeMap::new(),
            }),
            McpConfigurationEligibility::Complete { revision, values } => {
                selected.push(EligibleMcpMember {
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

/// Finds one eligible member by canonical identity.
pub(crate) fn find_eligible_member(
    catalog: &impl SessionMcpCatalog,
    configurations: &impl SessionMcpConfigurationSource,
    plugin_id: &ora_domain::PluginId,
) -> Result<Option<EligibleMcpMember>, SessionMcpError> {
    Ok(
        effective_members(catalog, configurations, &SessionMcpSelection::Automatic)?
            .into_iter()
            .find(|member| member.candidate.plugin_id == *plugin_id),
    )
}
