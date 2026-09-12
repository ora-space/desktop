//! A session's selection restricts both MCP delivery and live revision observation.

use std::collections::BTreeSet;

/// Ordinary chats discover eligible plugins; workflows supply a frozen, explicit allowlist.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum SessionMcpSelection {
    #[default]
    Automatic,
    Explicit(BTreeSet<String>),
}

impl SessionMcpSelection {
    /// Stable diagnostic label that reveals no MCP configuration values.
    pub(crate) const fn mode(&self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Explicit(_) => "explicit",
        }
    }
}

/// Supplies durable session intent without making the runtime depend on its owning use case.
///
/// Implementations must return an error for unreadable ownership metadata, rather than widening
/// an explicit selection to automatic discovery. The shared, nongeneric actor manager stores
/// this injected policy behind an Arc so replacement runtimes need no workflow implementation.
pub(crate) trait SessionMcpSelectionSource: Send + Sync {
    /// Resolves the selection owned by an existing session before attaching or rebinding it.
    fn selection_for(
        &self,
        session_id: &ora_domain::SessionId,
    ) -> Result<SessionMcpSelection, crate::BackendError>;
}

#[cfg(test)]
impl SessionMcpSelectionSource for SessionMcpSelection {
    /// Keeps runtime-only fixtures independent of external ownership repositories.
    fn selection_for(
        &self,
        _session_id: &ora_domain::SessionId,
    ) -> Result<SessionMcpSelection, crate::BackendError> {
        Ok(self.clone())
    }
}
