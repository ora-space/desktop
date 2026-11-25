//! Host implementations for a runtime that keeps nothing beyond its process and serves no MCP.
//!
//! Hosts whose sessions never outlive the process can reuse the memory store. Hosts with no MCP
//! configuration can separately compose the empty setup; production Node sessions provide their
//! own explicit setup. The runtime tests reuse both adapters.

use crate::RuntimeError;
use crate::host::{SessionSetup, SessionStore, SkillCatalogSnapshot, SkillCatalogSource};
use crate::session_setup::{AgentSessionMcpCapabilities, SessionMcpRevision, SessionMcpSnapshot};
use ora_domain::{
    AgentRef, HistoryState, PluginId, Session, SessionId, SessionMcpSelection, SessionStatus,
    SessionTitle, WorkspaceId,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;

/// The only failure the in-memory store reports: the addressed row is not visible.
#[derive(Debug, Error)]
#[error("session {0} is not stored")]
pub struct MissingSession(String);

/// Session rows in insertion order, with soft deletion kept as a flag like the durable store.
#[derive(Clone, Default)]
pub struct MemorySessionStore {
    rows: Arc<Mutex<Vec<Session>>>,
}

impl MemorySessionStore {
    /// Replaces one visible row through `update`, returning the stored result.
    fn update(
        &self,
        session_id: &SessionId,
        update: impl FnOnce(Session) -> Session,
    ) -> Result<Session, MissingSession> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let row = rows
            .iter_mut()
            .find(|row| row.id == *session_id && !row.audit_fields.is_deleted)
            .ok_or_else(|| MissingSession(session_id.to_string()))?;
        *row = update(row.clone());
        Ok(row.clone())
    }
}

impl SessionStore for MemorySessionStore {
    type Error = MissingSession;

    fn create_session(&self, session: Session) -> Result<Session, MissingSession> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session.clone());
        Ok(session)
    }

    fn find_session(&self, session_id: &SessionId) -> Result<Option<Session>, MissingSession> {
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|row| row.id == *session_id && !row.audit_fields.is_deleted)
            .cloned())
    }

    fn list_sessions(&self) -> Result<Vec<Session>, MissingSession> {
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|row| !row.audit_fields.is_deleted)
            .cloned()
            .collect())
    }

    fn update_session_title(
        &self,
        session_id: &SessionId,
        title: &SessionTitle,
        now: i64,
    ) -> Result<Session, MissingSession> {
        self.update(session_id, |row| {
            let mut row = row.with_title(Some(title.clone()));
            row.audit_fields.updated_at = now;
            row
        })
    }

    fn update_session_status(
        &self,
        session_id: &SessionId,
        status: SessionStatus,
        now: i64,
    ) -> Result<Session, MissingSession> {
        self.update(session_id, |row| row.with_status(status, now))
    }

    fn update_session_binding(
        &self,
        session_id: &SessionId,
        agent_ref: AgentRef,
        agent_session_id: &str,
        now: i64,
    ) -> Result<Session, MissingSession> {
        self.update(session_id, |row| {
            row.with_binding(agent_ref, agent_session_id, now)
        })
    }

    fn update_session_history_state(
        &self,
        session_id: &SessionId,
        history_state: &HistoryState,
        now: i64,
    ) -> Result<Session, MissingSession> {
        self.update(session_id, |row| {
            row.with_history_state(history_state.clone(), now)
        })
    }

    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, MissingSession> {
        Ok(self
            .update(session_id, |mut row| {
                row.audit_fields.is_deleted = true;
                row.audit_fields.updated_at = deleted_at;
                row
            })
            .is_ok())
    }
}

/// Resolves every session to an empty MCP set.
#[derive(Clone)]
pub struct NoSessionMcp {
    selection: SessionMcpSelection,
}

impl Default for NoSessionMcp {
    fn default() -> Self {
        Self {
            selection: SessionMcpSelection::Automatic,
        }
    }
}

impl SessionSetup for NoSessionMcp {
    fn with_selection(&self, selection: SessionMcpSelection) -> Self {
        Self { selection }
    }

    fn selection(&self) -> &SessionMcpSelection {
        &self.selection
    }

    fn desired_mcp_revision(&self) -> Result<SessionMcpRevision, RuntimeError> {
        Ok(SessionMcpRevision::default())
    }

    fn resolve_mcp(
        &self,
        _cwd: &Path,
        _capabilities: AgentSessionMcpCapabilities,
    ) -> Result<SessionMcpSnapshot, RuntimeError> {
        Ok(SessionMcpSnapshot::new(
            Vec::new(),
            SessionMcpRevision::default(),
        ))
    }

    fn observe_mcp_health(&self, _session_id: &SessionId, _cwd: &Path) {}

    fn plugin_id_for_agent(&self, agent_ref: &AgentRef) -> Option<PluginId> {
        PluginId::parse(agent_ref.as_str()).ok()
    }
}

/// Resolves every workspace to an empty Skill catalog and no additional readable roots.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSkillCatalog;

impl SkillCatalogSource for NoSkillCatalog {
    fn skill_catalog(
        &self,
        _workspace_id: &WorkspaceId,
    ) -> Result<SkillCatalogSnapshot, RuntimeError> {
        Ok(SkillCatalogSnapshot::default())
    }

    fn extra_read_roots(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn no_skill_catalog_returns_empty_entries_and_roots() {
        let catalog = NoSkillCatalog;
        assert_eq!(
            catalog
                .skill_catalog(&WorkspaceId::new("workspace-1"))
                .expect("NoSkillCatalog never fails"),
            SkillCatalogSnapshot::default()
        );
        assert_eq!(catalog.extra_read_roots(), Vec::<PathBuf>::new());
    }
}
