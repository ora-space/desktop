//! MCP desired-row mapping for the HTTP-only profile.
//!
//! These mappers mirror the Skill row mappers in the parent module but reconstruct a
//! [`DesiredMcpState`] whose payload never carries a Setting value, so a leak here can only ever
//! surface an environment-variable *reference*. The six functions live in their own module so the
//! MCP profile does not grow the Skill/general mapping file; they are re-exported back into the
//! parent ([`super`]) so every existing call site keeps resolving `mapping::` paths unchanged.

use crate::DatabaseError;
use ora_domain::{Namespace, WorkspaceId};
use ora_effect::{DesiredMcpState, McpSelectionKey};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

use super::effect_json_error;

/// Reconstructs one persisted MCP desired row from its plaintext-free revision payload.
///
/// The entire [`DesiredMcpState`] is the revision payload, so reconstruction is a single
/// deserialize plus an identity cross-check: the source-row `namespace`/`identifier` must agree
/// with the payload's own selection identity, otherwise the row was hand-edited into the wrong
/// source. The payload never carries a Setting value, so this mapping cannot leak one.
pub(in crate::repository::effect) fn map_mcp_desired(
    row: &Row<'_>,
) -> Result<(McpSelectionKey, DesiredMcpState), DatabaseError> {
    let payload: String = row.get("payload_json")?;
    let state: DesiredMcpState = serde_json::from_str(&payload).map_err(effect_json_error)?;
    let namespace = Namespace::new(row.get::<_, String>("namespace")?)?;
    let identifier = row.get::<_, String>("identifier")?;
    if state.namespace != namespace || state.identifier != identifier {
        return Err(DatabaseError::CorruptEffectState(
            "desired MCP payload identity disagrees with its source row".to_string(),
        ));
    }
    Ok((state.selection_key(), state))
}

/// Reconstructs an MCP selection identity from common source columns.
pub(in crate::repository::effect) fn map_mcp_selection_key(
    row: &Row<'_>,
) -> Result<McpSelectionKey, DatabaseError> {
    Ok(McpSelectionKey::new(
        Namespace::new(row.get::<_, String>("namespace")?)?,
        row.get::<_, String>("identifier")?,
    ))
}

/// Loads the current exact MCP source state only when its head revision is marked available.
///
/// Mirrors [`super::load_active_source`] for the HTTP-only MCP profile: the head revision's
/// payload IS the plaintext-free desired state, so a single deserialize reconstructs it. MCP
/// sources are always plugin-installed, so `source_kind` is fixed to `plugin` rather than read
/// from the key. Returns `None` when the source is retired or its head revision is unavailable,
/// which lets the replace loop report `SourceUnavailableMcp` exactly as the Skill loop reports
/// `SourceUnavailable`.
pub(in crate::repository::effect) fn load_mcp_active_source(
    connection: &Connection,
    selection_key: &McpSelectionKey,
) -> Result<Option<DesiredMcpState>, DatabaseError> {
    connection
        .query_row(
            "SELECT sources.namespace, sources.identifier, revisions.payload_json
             FROM effect_sources sources
             JOIN effect_source_heads heads ON heads.source_id = sources.id
             JOIN effect_source_revisions revisions ON revisions.id = heads.revision_id
             WHERE sources.effect_kind = 'mcp' AND sources.source_kind = 'plugin'
               AND sources.namespace = ?1 AND sources.identifier = ?2
               AND sources.lifecycle = 'active' AND revisions.availability = 'available'",
            params![
                selection_key.namespace.as_ref(),
                selection_key.identifier.as_str(),
            ],
            |row| {
                map_mcp_desired(row)
                    .map(|(_, desired)| desired)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })
            },
        )
        .optional()
        .map_err(Into::into)
}

/// Inserts one already normalized MCP desired row.
///
/// Mirrors [`super::insert_desired`]; the `_desired` payload is already stored as the revision
/// payload at publish time, so the desired row only points at the active head revision by
/// selection identity. `source_kind` is fixed to `plugin` because MCP sources are always
/// plugin-installed.
pub(in crate::repository::effect) fn insert_mcp_desired(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    selection_key: &McpSelectionKey,
    _desired: &DesiredMcpState,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO workspace_effect_desired_items (
             id, workspace_id, source_id, revision_id, created_at, updated_at
         )
         SELECT ?1, ?2, sources.id, heads.revision_id, ?5, ?5
         FROM effect_sources sources
         JOIN effect_source_heads heads ON heads.source_id = sources.id
         WHERE sources.effect_kind = 'mcp' AND sources.source_kind = 'plugin'
           AND sources.namespace = ?3 AND sources.identifier = ?4",
        params![
            Uuid::new_v4().to_string(),
            workspace_id.as_ref(),
            selection_key.namespace.as_ref(),
            selection_key.identifier.as_str(),
            updated_at,
        ],
    )?;
    Ok(())
}

/// Coalesces sequential MCP source updates by stable selection identity.
///
/// Mirrors [`super::upsert_propagation_request`]; the MCP revision column carries the bound
/// `store.json` revision, so two resolved-value sets at the same plugin version stay distinct
/// and propagate independently.
pub(in crate::repository::effect) fn upsert_mcp_propagation_request(
    transaction: &Transaction<'_>,
    selection_key: &McpSelectionKey,
    revision: u64,
    requested_at: i64,
) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO effect_propagation_requests (
             source_id, head_revision_id, request_token, attempt_count,
             requested_at, not_before_at, updated_at
         )
         SELECT sources.id, revisions.id, ?4, 0, ?5, ?5, ?5
         FROM effect_sources sources
         JOIN effect_source_revisions revisions ON revisions.source_id = sources.id
         WHERE sources.effect_kind = 'mcp' AND sources.source_kind = 'plugin'
           AND sources.namespace = ?1 AND sources.identifier = ?2 AND revisions.revision = ?3
         ON CONFLICT(source_id) DO UPDATE SET
             head_revision_id = excluded.head_revision_id, request_token = excluded.request_token,
             attempt_count = 0, requested_at = excluded.requested_at,
             not_before_at = excluded.not_before_at, updated_at = excluded.updated_at",
        params![
            selection_key.namespace.as_ref(),
            selection_key.identifier.as_str(),
            revision.to_string(),
            Uuid::new_v4().to_string(),
            requested_at,
        ],
    )?;
    Ok(())
}

/// Lists Workspaces that currently reference a stable MCP source selection.
pub(in crate::repository::effect) fn referenced_mcp_workspaces(
    connection: &Connection,
    selection_key: &McpSelectionKey,
) -> Result<Vec<WorkspaceId>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT desired.workspace_id
         FROM workspace_effect_desired_items desired
         JOIN effect_sources sources ON sources.id = desired.source_id
         WHERE sources.effect_kind = 'mcp' AND sources.source_kind = 'plugin'
           AND sources.namespace = ?1 AND sources.identifier = ?2
         ORDER BY desired.workspace_id",
    )?;
    statement
        .query_map(
            params![
                selection_key.namespace.as_ref(),
                selection_key.identifier.as_str()
            ],
            |row| Ok(WorkspaceId::new(row.get::<_, String>(0)?)),
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}
