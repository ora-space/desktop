use agent_client_protocol_schema::v1::Meta;
use serde_json::Value;

/// Metadata namespace used by OpenCode's inline child-session projection.
pub const OPENCODE_CHILD_SESSION_META_KEY: &str = "opencode/child-session";

/// Metadata namespace recorded for `_subagent/update` extension notifications.
pub const SUBAGENT_UPDATE_META_KEY: &str = "_subagent/update";

/// Reports whether `meta` names any recognized child-projection namespace, even when cleared.
///
/// Callers that track per-tool ownership across metadata-free deltas need to distinguish an
/// omitted namespace (inherit previous ownership) from an explicit `null` or malformed value
/// (reset ownership back to the parent).
pub fn has_child_source_namespace(meta: Option<&Meta>) -> bool {
    meta.is_some_and(|meta| {
        meta.contains_key(OPENCODE_CHILD_SESSION_META_KEY)
            || meta.contains_key(SUBAGENT_UPDATE_META_KEY)
    })
}

/// Identifies a valid child projection; cleared, conflicting, or malformed sources belong to the parent.
///
/// Transcript attribution and prompt liveness share this one rule, so a source history calls
/// the parent's can never hold a different ownership in the inactivity timer.
pub fn child_source_id(meta: Option<&Meta>) -> Option<&str> {
    let meta = meta?;
    match (
        meta.get(OPENCODE_CHILD_SESSION_META_KEY),
        meta.get(SUBAGENT_UPDATE_META_KEY),
    ) {
        (Some(opencode), None) => opencode_child_source_id(opencode),
        (None, Some(subagent)) => subagent_update_source_id(subagent),
        (Some(opencode), Some(subagent)) => {
            let id = opencode_child_source_id(opencode)?;
            (subagent_update_source_id(subagent) == Some(id)).then_some(id)
        }
        (None, None) => None,
    }
}

/// Validates OpenCode's `_meta["opencode/child-session"]` projection payload.
fn opencode_child_source_id(source: &Value) -> Option<&str> {
    let id = source
        .get("id")?
        .as_str()
        .filter(|id| !id.trim().is_empty())?;
    source
        .get("parentID")?
        .as_str()
        .filter(|parent| !parent.trim().is_empty())?;
    // JavaScript parses 1 and 1.0 as the same safe integer; replay must accept both.
    let depth = source.get("depth")?.as_f64()?;
    if !depth.is_finite() || depth < 1.0 || depth.fract() != 0.0 || depth > 9_007_199_254_740_991.0
    {
        return None;
    }
    Some(id)
}

/// Validates `_meta["_subagent/update"]` attribution persisted from a `SubagentProgressNotification`.
fn subagent_update_source_id(source: &Value) -> Option<&str> {
    let object = source.as_object()?;
    let task_id = object
        .get("taskId")?
        .as_str()
        .filter(|id| !id.trim().is_empty())?;
    object
        .get("sessionId")?
        .as_str()
        .filter(|session_id| !session_id.trim().is_empty())?;
    if !object
        .get("toolCallId")
        .is_none_or(|value| value.is_null() || value.is_string())
        || !object
            .get("agentType")
            .is_none_or(|value| value.is_null() || value.is_string())
    {
        return None;
    }
    Some(task_id)
}

/// Retains reported tool context while assembling one durable snapshot.
///
/// Metadata namespaces are opaque replacement values, not a recursive patch protocol.
/// Omitted namespaces survive; explicit null remains authoritative.
pub(super) fn retain_tool_metadata(target: &mut Option<Meta>, incoming: Option<&Meta>) {
    let Some(incoming) = incoming else {
        return;
    };
    let target = target.get_or_insert_with(Meta::new);
    for (key, value) in incoming {
        target.insert(key.clone(), value.clone());
    }
}
