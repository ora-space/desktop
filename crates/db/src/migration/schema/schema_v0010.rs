use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE effect_operations ADD COLUMN detected_at INTEGER;
UPDATE effect_operations SET detected_at = updated_at WHERE phase = 'recovery_required';

CREATE TRIGGER effect_operations_recovery_time_insert
BEFORE INSERT ON effect_operations
WHEN (NEW.phase = 'recovery_required') != (NEW.detected_at IS NOT NULL)
  OR NEW.detected_at < NEW.prepared_at
BEGIN
    SELECT RAISE(ABORT, 'invalid Effect recovery detection time');
END;

CREATE TRIGGER effect_operations_recovery_time_update
BEFORE UPDATE ON effect_operations
WHEN (NEW.phase = 'recovery_required') != (NEW.detected_at IS NOT NULL)
  OR NEW.detected_at < NEW.prepared_at
BEGIN
    SELECT RAISE(ABORT, 'invalid Effect recovery detection time');
END;

DROP TRIGGER effect_scopes_after_workspace_insert;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TRIGGER effect_operations_recovery_time_insert;
DROP TRIGGER effect_operations_recovery_time_update;
UPDATE effect_operations SET updated_at = detected_at WHERE phase = 'recovery_required';
ALTER TABLE effect_operations DROP COLUMN detected_at;

CREATE TRIGGER effect_scopes_after_workspace_insert
AFTER INSERT ON workspaces
BEGIN
    INSERT INTO effect_scopes (
        id, scope_kind, workspace_id, lifecycle, generation, created_at, updated_at
    ) VALUES (
        'workspace:' || NEW.id, 'workspace', NEW.id, 'active', 0, NEW.created_at, NEW.updated_at
    );
END;
"#];

/// Separates recovery evidence from audit time without rewriting existing Effect identities.
pub fn migration() -> Migration {
    Migration::new("0010", UP_STATEMENTS, DOWN_STATEMENTS)
}
