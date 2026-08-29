//! Host-side MCP Configuration Capability negotiation, snapshot protocol, and receipt checks.

mod invoke;
mod receipt;
mod snapshot;

#[cfg(test)]
mod tests;

// Snapshot, receipt, and invoke are the Host wire adapter. The Effect worker (#489) is the first
// runtime caller; this ticket keeps that cutover out of scope, so lib clippy would otherwise treat
// a complete protocol as dead code.
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
pub(crate) use invoke::{ConfigureWorkspaceError, ConfigureWorkspaceRuntime, configure_workspace};
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
pub(crate) use receipt::{
    ExpectedManagedMcp, ExpectedReceiptCoverage, McpConfigurationReceipt, ReceiptValidationError,
    parse_mcp_configuration_receipt, validate_mcp_configuration_receipt,
};
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
pub(crate) use snapshot::{
    DesiredResolvedMcp, PreparedMcpConfiguration, ResolvedMcpTransport, SnapshotRequestError,
    UnsupportedMcp, prepare_mcp_configuration_snapshot, snapshot_request_json,
};

use ora_effect::{AgentCapabilityRevision, Digest};
use ora_plugin_runtime::{
    CONFIGURE_WORKSPACE_METHOD, McpConfigurationCapability, McpConfigurationCapabilityIssue,
    McpConfigurationRegistration, PluginRegistration,
};

/// Outcome of pairing a registration capability with `agent/configureWorkspace`.
///
/// `Unsupported` is the older-plugin case: every Ready MCP is target-specific Unsupported.
/// `Disabled` keeps conversation available while refusing MCP materialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NegotiatedMcpConfiguration {
    Unsupported,
    Disabled {
        reason: McpConfigurationDisableReason,
    },
    Enabled(McpConfigurationCapability),
}

/// Why a present or partial MCP declaration cannot be used for materialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum McpConfigurationDisableReason {
    Invalid(McpConfigurationCapabilityIssue),
    CapabilityWithoutHandler,
    HandlerWithoutCapability,
}

impl McpConfigurationDisableReason {
    /// Stable public error code that never includes capability payload bytes.
    pub(crate) fn error_code(&self) -> &'static str {
        match self {
            Self::Invalid(issue) => issue.error_code(),
            Self::CapabilityWithoutHandler | Self::HandlerWithoutCapability => {
                "mcp_capability_invalid"
            }
        }
    }
}

impl NegotiatedMcpConfiguration {
    /// Returns whether the Host may send `agent/configureWorkspace`.
    pub(crate) fn enables_materialization(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    /// Returns the enabled capability when MCP materialization is available.
    ///
    /// The Agent Target worker (#489) is the first runtime caller of this accessor; attach already
    /// stores the full negotiation outcome on the declaration snapshot.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn capability(&self) -> Option<&McpConfigurationCapability> {
        match self {
            Self::Enabled(capability) => Some(capability),
            Self::Unsupported | Self::Disabled { .. } => None,
        }
    }

    /// Returns the stable disable code when materialization is refused.
    pub(crate) fn disable_error_code(&self) -> Option<&'static str> {
        match self {
            Self::Disabled { reason } => Some(reason.error_code()),
            Self::Unsupported | Self::Enabled(_) => None,
        }
    }
}

/// Process-local Agent Effect declaration stored in the single convergence snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentPluginEffectDeclaration {
    pub skill_surfaces: Vec<ora_effect::FilesystemSkillSurface>,
    pub mcp_configuration: NegotiatedMcpConfiguration,
    pub capability_revision: AgentCapabilityRevision,
}

/// Pairs capability and handler without failing the baseline Agent conversation contract.
pub(crate) fn negotiate_mcp_configuration(
    registration: &PluginRegistration,
) -> NegotiatedMcpConfiguration {
    let has_handler = registration.methods.contains(CONFIGURE_WORKSPACE_METHOD);
    match &registration.mcp_configuration {
        McpConfigurationRegistration::Absent if has_handler => {
            NegotiatedMcpConfiguration::Disabled {
                reason: McpConfigurationDisableReason::HandlerWithoutCapability,
            }
        }
        McpConfigurationRegistration::Absent => NegotiatedMcpConfiguration::Unsupported,
        McpConfigurationRegistration::Invalid(issue) => NegotiatedMcpConfiguration::Disabled {
            reason: McpConfigurationDisableReason::Invalid(issue.clone()),
        },
        McpConfigurationRegistration::Declared(capability) if has_handler => {
            NegotiatedMcpConfiguration::Enabled(capability.clone())
        }
        McpConfigurationRegistration::Declared(_) => NegotiatedMcpConfiguration::Disabled {
            reason: McpConfigurationDisableReason::CapabilityWithoutHandler,
        },
    }
}

/// Binds the exact plugin version to the canonical digest of the declared capability.
///
/// Version-only and digest-only changes each produce a new revision so later workers can wake
/// every Agent Target after an upgrade or a repaired declaration.
pub(crate) fn agent_capability_revision(
    plugin_version: &str,
    registration: &McpConfigurationRegistration,
) -> AgentCapabilityRevision {
    AgentCapabilityRevision::bind(
        plugin_version,
        &Digest::sha256(&registration.canonical_declaration_bytes()),
    )
}
