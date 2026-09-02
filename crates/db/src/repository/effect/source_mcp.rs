use super::source::{advance_changed_scopes, collect_referencing_scopes};
use crate::DatabaseError;
use ora_domain::PluginId;
use ora_effect::{
    Digest, EffectKind, EffectRevisionId, McpParameters, McpTemplateDefinition, SourceRevisionKey,
    TargetSelector, ValidatedEffectDefinition, ValidatedEffectParameters,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;

/// Complete secret-free MCP source projection produced from one configured installed plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpSourceProjection {
    pub plugin_id: PluginId,
    pub revision_key: SourceRevisionKey,
    pub definition: McpTemplateDefinition,
}

/// Summarizes whether all active Agent Targets have consumed one MCP plugin's current intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpProjectionStatus {
    Projecting,
    Current,
    Blocked,
}

impl super::SqliteEffectRepository {
    /// Aggregates current MCP projection evidence without exposing Target or Condition details.
    pub fn plugin_mcp_projection_status(
        &self,
        plugin_id: &PluginId,
    ) -> Result<McpProjectionStatus, DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT status.phase, status.desired_generation, status.ready_generation,
                        EXISTS (
                            SELECT 1 FROM effect_conditions condition
                            WHERE condition.impact = 'blocking'
                              AND ((condition.owner_kind = 'target'
                                    AND condition.owner_id = target.id)
                                OR (condition.owner_kind = 'resource'
                                    AND condition.owner_id IN (
                                        SELECT binding.resource_id
                                        FROM effect_target_resource_bindings binding
                                        WHERE binding.target_id = target.id
                                    )))
                        ) AS blocked
                 FROM effect_targets target
                 JOIN effect_target_status status ON status.target_id = target.id
                 WHERE target.lifecycle = 'active'
                   AND EXISTS (
                       SELECT 1
                       FROM effect_target_resource_bindings binding
                       JOIN effect_resources resource ON resource.id = binding.resource_id
                       WHERE binding.target_id = target.id
                         AND resource.lifecycle = 'active'
                         AND resource.materialization_format IN (
                             'ora/opencode-mcp-config.v1', 'ora/claude-mcp-config.v1'
                         )
                   )
                   AND EXISTS (
                       SELECT 1
                       FROM effect_desired_effects desired
                       JOIN effect_revisions revision ON revision.id = desired.revision_id
                       JOIN effect_sources source ON source.id = revision.source_id
                       WHERE desired.scope_id = target.scope_id
                         AND source.effect_kind = ?1
                         AND source.source_kind = 'plugin'
                         AND source.namespace = ?2
                         AND source.identifier = ?3
                   )
                 ORDER BY target.id",
            )?;
            let rows = statement
                .query_map(
                    params![
                        EffectKind::mcp().as_str(),
                        plugin_id.namespace(),
                        plugin_id.name()
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, bool>(3)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            if rows.iter().any(|(phase, _, _, blocked)| {
                *blocked || matches!(phase.as_str(), "current_with_issues" | "recovery_required")
            }) {
                return Ok(McpProjectionStatus::Blocked);
            }
            if rows
                .iter()
                .all(|(phase, desired, ready, _)| phase == "current" && ready == desired)
            {
                Ok(McpProjectionStatus::Current)
            } else {
                Ok(McpProjectionStatus::Projecting)
            }
        })
    }

    /// Publishes one complete MCP template or retires that plugin's current MCP source.
    pub fn replace_plugin_mcp(
        &self,
        plugin_id: &PluginId,
        projection: Option<&McpSourceProjection>,
        updated_at: i64,
    ) -> Result<(), DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut changed_scopes = BTreeSet::new();
            match projection {
                Some(projection) => {
                    publish(&transaction, projection, updated_at, &mut changed_scopes)?
                }
                None => retire(&transaction, plugin_id, updated_at, &mut changed_scopes)?,
            }
            advance_changed_scopes(&transaction, &changed_scopes, updated_at)?;
            transaction.commit()?;
            Ok(())
        })
    }

    /// Retires installed-plugin MCP sources absent from the current discovery snapshot.
    pub fn retire_unlisted_plugin_mcps(
        &self,
        active_plugin_ids: &BTreeSet<String>,
        updated_at: i64,
    ) -> Result<(), DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut statement = transaction.prepare(
                "SELECT namespace, identifier FROM effect_sources
                 WHERE effect_kind = ?1 AND source_kind = 'plugin' AND lifecycle = 'active'
                 ORDER BY namespace, identifier",
            )?;
            let sources = statement
                .query_map(params![EffectKind::mcp().as_str()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            let mut changed_scopes = BTreeSet::new();
            for (namespace, identifier) in sources {
                let canonical = format!("{namespace}/{identifier}");
                let plugin_id = PluginId::parse(&canonical)?;
                if !active_plugin_ids.contains(&plugin_id.canonical()) {
                    retire(&transaction, &plugin_id, updated_at, &mut changed_scopes)?;
                }
            }
            advance_changed_scopes(&transaction, &changed_scopes, updated_at)?;
            transaction.commit()?;
            Ok(())
        })
    }
}

/// Publishes an immutable secret-free definition and updates current Desired references.
fn publish(
    connection: &rusqlite::Connection,
    projection: &McpSourceProjection,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let existing_source = find_source(connection, &projection.plugin_id)?;
    let source_id = existing_source
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO effect_sources (
             id, effect_kind, source_kind, namespace, identifier, lifecycle,
             publication_state, published_revision_id, created_at, updated_at
         ) VALUES (?1, ?2, 'plugin', ?3, ?4, 'active', 'unpublished', NULL, ?5, ?5)
         ON CONFLICT(effect_kind, source_kind, namespace, identifier) DO UPDATE SET
             lifecycle = 'active', updated_at = MAX(effect_sources.updated_at, excluded.updated_at)",
        params![
            &source_id,
            EffectKind::mcp().as_str(),
            projection.plugin_id.namespace(),
            projection.plugin_id.name(),
            updated_at,
        ],
    )?;
    let definition = ValidatedEffectDefinition::Mcp(projection.definition.clone());
    let definition_json = super::mapping::effect_json(&definition)?;
    let digest = Digest::sha256(definition_json.as_bytes());
    let existing_revision = connection
        .query_row(
            "SELECT id, definition_json, digest FROM effect_revisions
             WHERE source_id = ?1 AND revision_key = ?2",
            params![&source_id, projection.revision_key.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let revision_id = match existing_revision {
        Some((id, stored_definition, stored_digest)) => {
            if stored_definition != definition_json || stored_digest != digest.as_str() {
                return Err(DatabaseError::CorruptEffectState(
                    "an immutable MCP Effect revision changed content".to_string(),
                ));
            }
            connection.execute(
                "UPDATE effect_revisions SET availability = 'available', unavailable_reason = NULL,
                    updated_at = MAX(updated_at, ?2) WHERE id = ?1",
                params![&id, updated_at],
            )?;
            id
        }
        None => {
            let id = EffectRevisionId::random();
            connection.execute(
                "INSERT INTO effect_revisions (
                     id, source_id, revision_key, definition_kind, definition_version,
                     definition_json, digest, availability, unavailable_reason, created_at,
                     updated_at
                 ) VALUES (?1, ?2, ?3, 'mcp', 1, ?4, ?5, 'available', NULL, ?6, ?6)",
                params![
                    id.as_str(),
                    &source_id,
                    projection.revision_key.as_str(),
                    &definition_json,
                    digest.as_str(),
                    updated_at,
                ],
            )?;
            id.to_string()
        }
    };
    let previous_revision = connection.query_row(
        "SELECT published_revision_id FROM effect_sources WHERE id = ?1",
        params![&source_id],
        |row| row.get::<_, Option<String>>(0),
    )?;
    connection.execute(
        "UPDATE effect_sources SET publication_state = 'published', published_revision_id = ?2,
            updated_at = MAX(updated_at, ?3) WHERE id = ?1",
        params![&source_id, &revision_id, updated_at],
    )?;
    if existing_source.is_none() {
        install_mcp_in_all_scopes(connection, &revision_id, updated_at, changed_scopes)?;
    } else if previous_revision.as_deref() != Some(revision_id.as_str()) {
        collect_referencing_scopes(connection, &source_id, changed_scopes)?;
        connection.execute(
            "UPDATE effect_desired_effects SET revision_id = ?2, updated_at = MAX(updated_at, ?3)
             WHERE revision_id IN (SELECT id FROM effect_revisions WHERE source_id = ?1)",
            params![&source_id, &revision_id, updated_at],
        )?;
    }
    Ok(())
}

/// Removes Desired intent before retiring a missing or incomplete MCP source.
fn retire(
    connection: &rusqlite::Connection,
    plugin_id: &PluginId,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let Some(source_id) = find_source(connection, plugin_id)? else {
        return Ok(());
    };
    collect_referencing_scopes(connection, &source_id, changed_scopes)?;
    connection.execute(
        "DELETE FROM effect_desired_effects
         WHERE revision_id IN (SELECT id FROM effect_revisions WHERE source_id = ?1)",
        params![&source_id],
    )?;
    connection.execute(
        "UPDATE effect_sources SET lifecycle = 'retired', updated_at = MAX(updated_at, ?2)
         WHERE id = ?1",
        params![&source_id, updated_at],
    )?;
    connection.execute(
        "UPDATE effect_revisions SET availability = 'unavailable',
            unavailable_reason = 'source_retired', updated_at = MAX(updated_at, ?2)
         WHERE source_id = ?1",
        params![&source_id, updated_at],
    )?;
    Ok(())
}

/// Inserts one MCP Desired Effect into every currently active Scope.
fn install_mcp_in_all_scopes(
    connection: &rusqlite::Connection,
    revision_id: &str,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let parameters =
        super::mapping::effect_json(&ValidatedEffectParameters::Mcp(McpParameters::default()))?;
    let selector = super::mapping::effect_json(&TargetSelector::default())?;
    let mut statement = connection.prepare(
        "SELECT id, updated_at FROM effect_scopes WHERE lifecycle = 'active' ORDER BY id",
    )?;
    let scopes = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (scope_id, scope_updated_at) in scopes {
        let desired_updated_at = scope_updated_at.max(updated_at);
        connection.execute(
            "INSERT INTO effect_desired_effects (
                 id, scope_id, revision_id, parameters_kind, parameters_version,
                 parameters_json, selector_version, selector_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'mcp', 1, ?4, 1, ?5, ?6, ?6)",
            params![
                ora_effect::DesiredEffectIdentity::random().as_str(),
                &scope_id,
                revision_id,
                &parameters,
                &selector,
                desired_updated_at,
            ],
        )?;
        changed_scopes.insert(scope_id);
    }
    Ok(())
}

/// Finds one MCP source by its normalized plugin identity.
fn find_source(
    connection: &rusqlite::Connection,
    plugin_id: &PluginId,
) -> Result<Option<String>, DatabaseError> {
    connection
        .query_row(
            "SELECT id FROM effect_sources
             WHERE effect_kind = ?1 AND source_kind = 'plugin'
               AND namespace = ?2 AND identifier = ?3",
            params![
                EffectKind::mcp().as_str(),
                plugin_id.namespace(),
                plugin_id.name(),
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}
