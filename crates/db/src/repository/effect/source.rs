use super::mapping::{effect_json, generation_from_sql, generation_to_sql};
use crate::DatabaseError;
use ora_effect::{
    DesiredEffectIdentity, Digest, EffectKind, EffectRevisionId, EffectScopeId, Fingerprint,
    Generation, SkillDefinition, SkillParameters, SkillSourceKey, SkillSourceKind,
    SourceRevisionKey, TargetSelector, ValidatedEffectDefinition, ValidatedEffectParameters,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;
use std::path::PathBuf;
use uuid::Uuid;

/// Complete validated source input used by Skill CRUD and startup reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PublishedSkillRevision {
    pub source: SkillSourceKey,
    pub revision_key: SourceRevisionKey,
    pub skill_md_digest: Digest,
    pub package_fingerprint: Fingerprint,
    pub package_root: PathBuf,
}

/// Publishes one immutable Skill revision and updates current Desired references in-place.
pub(crate) fn publish_skill_revision(
    connection: &Connection,
    publication: &PublishedSkillRevision,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<EffectRevisionId, DatabaseError> {
    let existing_source = find_source(connection, &publication.source)?;
    let source_id = existing_source
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO effect_sources (
             id, effect_kind, source_kind, namespace, identifier, lifecycle,
             publication_state, published_revision_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', 'unpublished', NULL, ?6, ?6)
         ON CONFLICT(effect_kind, source_kind, namespace, identifier) DO UPDATE SET
             lifecycle = 'active', updated_at = MAX(effect_sources.updated_at, excluded.updated_at)",
        params![
            &source_id,
            EffectKind::skill().as_str(),
            source_kind_value(publication.source.source_kind),
            publication.source.namespace.as_ref(),
            publication.source.name.canonical(),
            updated_at,
        ],
    )?;

    let definition = ValidatedEffectDefinition::Skill(SkillDefinition {
        source: publication.source.clone(),
        skill_md_digest: publication.skill_md_digest.clone(),
        package_fingerprint: publication.package_fingerprint.clone(),
        package_root: publication.package_root.clone(),
    });
    let definition_json = effect_json(&definition)?;
    let revision_digest = Digest::sha256(definition_json.as_bytes());
    let existing_revision = connection
        .query_row(
            "SELECT id, definition_json, digest FROM effect_revisions
             WHERE source_id = ?1 AND revision_key = ?2",
            params![&source_id, publication.revision_key.as_str()],
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
            if stored_definition != definition_json || stored_digest != revision_digest.as_str() {
                return Err(DatabaseError::CorruptEffectState(
                    "an immutable Effect revision changed content".to_string(),
                ));
            }
            connection.execute(
                "UPDATE effect_revisions
                 SET availability = 'available', unavailable_reason = NULL,
                     updated_at = MAX(updated_at, ?2)
                 WHERE id = ?1",
                params![&id, updated_at],
            )?;
            id
        }
        None => {
            let id = Uuid::new_v4().to_string();
            connection.execute(
                "INSERT INTO effect_revisions (
                     id, source_id, revision_key, definition_kind, definition_version,
                     definition_json, digest, availability, unavailable_reason, created_at,
                     updated_at
                 ) VALUES (?1, ?2, ?3, 'skill', 1, ?4, ?5, 'available', NULL, ?6, ?6)",
                params![
                    &id,
                    &source_id,
                    publication.revision_key.as_str(),
                    &definition_json,
                    revision_digest.as_str(),
                    updated_at,
                ],
            )?;
            id
        }
    };
    let previous_revision = connection.query_row(
        "SELECT published_revision_id FROM effect_sources WHERE id = ?1",
        params![&source_id],
        |row| row.get::<_, Option<String>>(0),
    )?;
    connection.execute(
        "UPDATE effect_sources
         SET publication_state = 'published', published_revision_id = ?2,
             updated_at = MAX(updated_at, ?3)
         WHERE id = ?1",
        params![&source_id, &revision_id, updated_at],
    )?;

    if existing_source.is_none() {
        install_source_in_all_scopes(
            connection,
            &source_id,
            &revision_id,
            updated_at,
            changed_scopes,
        )?;
    } else if previous_revision.as_deref() != Some(revision_id.as_str()) {
        collect_referencing_scopes(connection, &source_id, changed_scopes)?;
        connection.execute(
            "UPDATE effect_desired_effects
             SET revision_id = ?2, updated_at = MAX(updated_at, ?3)
             WHERE revision_id IN (SELECT id FROM effect_revisions WHERE source_id = ?1)",
            params![&source_id, &revision_id, updated_at],
        )?;
    }
    Ok(EffectRevisionId::new(revision_id))
}

/// Seeds a newly created Scope from every active published source as one Desired replacement.
pub(crate) fn seed_scope_sources(
    connection: &Connection,
    scope: &EffectScopeId,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let scope_id = scope.storage_key();
    let selector = effect_json(&TargetSelector::default())?;
    let skill_parameters =
        effect_json(&ValidatedEffectParameters::Skill(SkillParameters::default()))?;
    let inserted = connection.execute(
        "INSERT INTO effect_desired_effects (
             id, scope_id, revision_id, parameters_kind, parameters_version, parameters_json,
             selector_version, selector_json, created_at, updated_at
         )
         SELECT lower(hex(randomblob(16))), ?1, sources.published_revision_id,
                ?6, 1, ?2, 1, ?3, ?4, ?4
         FROM effect_sources sources
         WHERE sources.effect_kind = ?5 AND sources.lifecycle = 'active'
           AND sources.publication_state = 'published'
           AND NOT EXISTS (
               SELECT 1 FROM effect_desired_effects desired
               JOIN effect_revisions revision ON revision.id = desired.revision_id
               WHERE desired.scope_id = ?1 AND revision.source_id = sources.id
           )",
        params![
            &scope_id,
            skill_parameters,
            selector,
            updated_at,
            EffectKind::skill().as_str(),
            "skill",
        ],
    )?;
    if inserted > 0 {
        changed_scopes.insert(scope_id);
    }
    Ok(())
}

/// Retires one source and removes only its current Desired intent from active Scopes.
pub(crate) fn retire_skill_source(
    connection: &Connection,
    source: &SkillSourceKey,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<bool, DatabaseError> {
    let Some(source_id) = find_source(connection, source)? else {
        return Ok(false);
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
        "UPDATE effect_revisions
         SET availability = 'unavailable', unavailable_reason = 'source_retired',
             updated_at = MAX(updated_at, ?2)
         WHERE source_id = ?1",
        params![&source_id, updated_at],
    )?;
    Ok(true)
}

/// Advances each changed complete Desired State exactly once and wakes every active Target.
pub(crate) fn advance_changed_scopes(
    connection: &Connection,
    changed_scopes: &BTreeSet<String>,
    updated_at: i64,
) -> Result<(), DatabaseError> {
    for scope_id in changed_scopes {
        let (current, scope_updated_at) = connection.query_row(
            "SELECT generation, updated_at FROM effect_scopes
             WHERE id = ?1 AND lifecycle = 'active'",
            params![scope_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let generation = generation_from_sql(current)?
            .next()
            .map_err(|error| DatabaseError::CorruptEffectState(error.to_string()))?;
        let generation_sql = generation_to_sql(generation)?;
        // A replayed Source timestamp may predate a Scope that did not exist when the Source was
        // created. Scope-local history begins no earlier than that Scope's current timestamp.
        let scope_updated_at = scope_updated_at.max(updated_at);
        connection.execute(
            "UPDATE effect_scopes SET generation = ?2, updated_at = ?3 WHERE id = ?1",
            params![scope_id, generation_sql, scope_updated_at],
        )?;
        connection.execute(
            "UPDATE effect_target_status
             SET desired_generation = MAX(desired_generation, ?2),
                 phase = CASE
                     WHEN phase IN ('retiring', 'recovery_required') THEN phase ELSE 'pending' END,
                 status_version = status_version + 1, updated_at = ?3
             WHERE target_id IN (
                 SELECT id FROM effect_targets WHERE scope_id = ?1 AND lifecycle = 'active'
             )",
            params![scope_id, generation_sql, scope_updated_at],
        )?;
        wake_scope_targets(
            connection,
            scope_id,
            generation,
            scope_updated_at,
            "desired_changed",
        )?;
        connection.execute(
            "INSERT INTO effect_audit_events (
                 id, scope_id, subject_kind, subject_id, event_kind, generation,
                 initiator_kind, initiator_id, payload_version, payload_json, occurred_at
             ) VALUES (?1, ?2, 'desired_state', ?2, 'desired_replaced', ?3,
                       'system', NULL, 1, '{}', ?4)",
            params![
                Uuid::new_v4().to_string(),
                scope_id,
                generation_sql,
                scope_updated_at,
            ],
        )?;
    }
    Ok(())
}

/// Coalesces one diagnostic wake reason while preserving a still-valid worker claim.
pub(super) fn wake_scope_targets(
    connection: &Connection,
    scope_id: &str,
    generation: Generation,
    updated_at: i64,
    reason: &str,
) -> Result<(), DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id FROM effect_targets
         WHERE scope_id = ?1 AND lifecycle IN ('active', 'retiring') ORDER BY id",
    )?;
    let targets = statement
        .query_map(params![scope_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for target in targets {
        super::declaration::upsert_target_wakeup(
            connection, &target, generation, updated_at, reason,
        )?;
    }
    Ok(())
}

/// Inserts one stable Desired Effect for a newly published source into every active Scope.
pub(super) fn install_source_in_all_scopes(
    connection: &Connection,
    source_id: &str,
    revision_id: &str,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let parameters = effect_json(&ValidatedEffectParameters::Skill(SkillParameters::default()))?;
    let selector = effect_json(&TargetSelector::default())?;
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
             ) VALUES (?1, ?2, ?3, 'skill', 1, ?4, 1, ?5, ?6, ?6)",
            params![
                DesiredEffectIdentity::random().as_str(),
                &scope_id,
                revision_id,
                &parameters,
                &selector,
                desired_updated_at,
            ],
        )?;
        changed_scopes.insert(scope_id);
    }
    let _ = source_id;
    Ok(())
}

/// Finds one stable Skill source independently of its current publication.
fn find_source(
    connection: &Connection,
    source: &SkillSourceKey,
) -> Result<Option<String>, DatabaseError> {
    connection
        .query_row(
            "SELECT id FROM effect_sources
             WHERE effect_kind = ?1 AND source_kind = ?2
               AND namespace = ?3 AND identifier = ?4",
            params![
                EffectKind::skill().as_str(),
                source_kind_value(source.source_kind),
                source.namespace.as_ref(),
                source.name.canonical(),
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

/// Collects every Scope whose current Desired State refers to one source.
pub(super) fn collect_referencing_scopes(
    connection: &Connection,
    source_id: &str,
    scopes: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT desired.scope_id
         FROM effect_desired_effects desired
         JOIN effect_revisions revision ON revision.id = desired.revision_id
         WHERE revision.source_id = ?1 ORDER BY desired.scope_id",
    )?;
    let rows = statement
        .query_map(params![source_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    scopes.extend(rows);
    Ok(())
}

/// Encodes the closed first-version Skill source kind for normalized persistence.
pub(super) fn source_kind_value(kind: SkillSourceKind) -> &'static str {
    match kind {
        SkillSourceKind::Local => "local",
        SkillSourceKind::Plugin => "plugin",
    }
}
