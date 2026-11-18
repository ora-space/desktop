//! Translates `_subagent/update` ACP extension notifications into attributed session updates.
//!
//! CodeAgent delivers subagent progress over the `_subagent/update` `extNotification` channel when
//! `CODEAGENT3_ACP_SUBAGENT_CHANNEL=true`, keeping subagent tool events out of `session/update`.
//! Normalizing those frames at the connection boundary lets routing, prompt liveness, durable
//! JSONL history, and replay share one `SessionUpdate` pipeline without widening public contracts.

use agent_client_protocol_schema::v1::{Meta, SessionNotification, SessionUpdate};
use ora_acp::ExtensionNotification;
use ora_history::SUBAGENT_UPDATE_META_KEY;
use ora_logging::ora_warn;
use serde_json::{Map, Value};

/// ACP extension method CodeAgent uses to stream subagent progress.
pub(super) const SUBAGENT_UPDATE_METHOD: &str = "_subagent/update";

/// Converts a recognized ACP extension notification into a routed `SessionNotification`.
///
/// Unrecognized `_`-prefixed methods are ignored quietly because ACP reserves that namespace for
/// arbitrary provider extensions. Malformed `_subagent/update` payloads are logged and dropped so
/// one bad extension frame cannot tear down every live session sharing the agent process.
pub(super) fn translate_extension_notification(
    extension: ExtensionNotification,
) -> Option<SessionNotification> {
    if extension.method != SUBAGENT_UPDATE_METHOD {
        return None;
    }
    match decode_subagent_notification(extension.params) {
        Ok(notification) => Some(notification),
        Err(reason) => {
            ora_warn!(
                method = SUBAGENT_UPDATE_METHOD,
                reason = %reason,
                "dropping malformed subagent extension notification"
            );
            None
        }
    }
}

/// Validates one `_subagent/update` payload, scopes inner tool ids, and attaches source metadata.
fn decode_subagent_notification(params: Value) -> Result<SessionNotification, String> {
    let mut object = match params {
        Value::Object(object) => object,
        _ => return Err("params must be a JSON object".to_string()),
    };
    let session_id = required_non_empty_string(&object, "sessionId")?;
    let task_id = required_non_empty_string(&object, "taskId")?;
    let parent_tool_call_id = optional_non_empty_string(&object, "toolCallId")?;
    let agent_type = optional_non_empty_string(&object, "agentType")?;

    let raw_update = object
        .remove("update")
        .ok_or_else(|| "missing `update` object".to_string())?;
    let mut update_object = match raw_update {
        Value::Object(map) => map,
        _ => return Err("`update` must be a JSON object".to_string()),
    };

    normalize_inner_update(&mut update_object, &task_id)?;

    let mut update: SessionUpdate = serde_json::from_value(Value::Object(update_object))
        .map_err(|error| format!("invalid inner sessionUpdate: {error}"))?;
    let attribution = build_source_metadata(
        &session_id,
        &task_id,
        parent_tool_call_id.as_deref(),
        agent_type.as_deref(),
    );
    attach_source_metadata(&mut update, attribution)?;

    Ok(SessionNotification::new(session_id, update))
}

/// Enforces allowed subagent update kinds and namespaces subagent-local tool call ids by `task_id`.
///
/// Subagent runs execute in their own model turns, where OpenAI-compatible gateways can reuse short
/// ids such as `call_0` across parent and sibling subagents. Prefixing the inner `toolCallId` with
/// `task_id` prevents collisions in `PromptLiveness`, `HistoryAssembler`, and UI item keys while
/// keeping `tool_call` and `tool_call_update` aligned within the same subagent task.
fn normalize_inner_update(update: &mut Map<String, Value>, task_id: &str) -> Result<(), String> {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing `update.sessionUpdate` string".to_string())?;

    match kind {
        "agent_message_chunk" | "agent_thought_chunk" | "plan" => Ok(()),
        "tool_call" => {
            scope_inner_tool_call_id(update, task_id)?;
            for nullable_key in ["content", "locations", "kind", "status"] {
                if matches!(update.get(nullable_key), Some(Value::Null)) {
                    update.remove(nullable_key);
                }
            }
            let missing_title = update
                .get("title")
                .and_then(Value::as_str)
                .is_none_or(|title| title.trim().is_empty());
            if missing_title {
                let fallback = update
                    .get("_meta")
                    .and_then(|meta| meta.pointer("/codeAgentCLI/toolName"))
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or("Tool call")
                    .to_string();
                update.insert("title".to_string(), Value::String(fallback));
            }
            Ok(())
        }
        "tool_call_update" => scope_inner_tool_call_id(update, task_id),
        other => Err(format!(
            "unsupported subagent sessionUpdate variant `{other}`"
        )),
    }
}

/// Prefixes the subagent's own `toolCallId` with `task_id` so it cannot collide with parent calls.
fn scope_inner_tool_call_id(update: &mut Map<String, Value>, task_id: &str) -> Result<(), String> {
    let inner_id = required_non_empty_string(update, "toolCallId")?;
    update.insert(
        "toolCallId".to_string(),
        Value::String(format!("{task_id}:{inner_id}")),
    );
    Ok(())
}

/// Builds the stable attribution payload stored under `_meta["_subagent/update"]`.
///
/// Per-frame `timestamp` is intentionally excluded so contiguous text and thought chunks from the
/// same subagent retain identical metadata and merge into one history record.
fn build_source_metadata(
    session_id: &str,
    task_id: &str,
    parent_tool_call_id: Option<&str>,
    agent_type: Option<&str>,
) -> Value {
    let mut source = Map::new();
    source.insert(
        "sessionId".to_string(),
        Value::String(session_id.to_string()),
    );
    source.insert("taskId".to_string(), Value::String(task_id.to_string()));
    if let Some(tool_call_id) = parent_tool_call_id {
        source.insert(
            "toolCallId".to_string(),
            Value::String(tool_call_id.to_string()),
        );
    }
    if let Some(agent_type) = agent_type {
        source.insert(
            "agentType".to_string(),
            Value::String(agent_type.to_string()),
        );
    }
    Value::Object(source)
}

/// Inserts `_meta["_subagent/update"]` while preserving provider-supplied `_meta` siblings.
fn attach_source_metadata(update: &mut SessionUpdate, source: Value) -> Result<(), String> {
    let meta = match update {
        SessionUpdate::AgentMessageChunk(chunk) | SessionUpdate::AgentThoughtChunk(chunk) => {
            &mut chunk.meta
        }
        SessionUpdate::ToolCall(call) => &mut call.meta,
        SessionUpdate::ToolCallUpdate(change) => &mut change.meta,
        SessionUpdate::Plan(plan) => &mut plan.meta,
        _ => return Err("unsupported decoded SessionUpdate variant".to_string()),
    };
    meta.get_or_insert_with(Meta::new)
        .insert(SUBAGENT_UPDATE_META_KEY.to_string(), source);
    Ok(())
}

/// Reads a required string field that must contain non-whitespace characters.
fn required_non_empty_string(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing or blank `{key}`"))
}

/// Reads an optional string field; blank strings and `null` normalize to `None`, while non-strings fail.
fn optional_non_empty_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("`{key}` must be a string when present")),
    }
}

#[cfg(test)]
mod tests {
    use super::translate_extension_notification;
    use crate::prompt_liveness::PromptLiveness;
    use agent_client_protocol_schema::v1::{SessionUpdate, StopReason, ToolCall, ToolCallStatus};
    use ora_acp::ExtensionNotification;
    use ora_history::{HistoryAssembler, HistoryRecord, child_source_id};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::time::Duration;

    fn subagent_ext(params: Value) -> ExtensionNotification {
        ExtensionNotification {
            method: "_subagent/update".to_string(),
            params,
        }
    }

    #[test]
    fn translates_message_thought_and_tool_lifecycle_with_stable_attribution() {
        let mut assembler = HistoryAssembler::new(0);
        // Parent starts the Agent/Task delegation call with id `call_1`.
        let parent_delegate = SessionUpdate::ToolCall(
            ToolCall::new("call_1", "Task: Explore").status(ToolCallStatus::InProgress),
        );
        assembler.push_update(&parent_delegate);

        // Subagent streams two text chunks with differing timestamps and its own `call_1` tool id.
        let frames = [
            json!({
                "sessionId": "ses-main",
                "taskId": "task-explore",
                "toolCallId": "call_1",
                "agentType": "Explore",
                "update": {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "Checking files" },
                },
                "timestamp": 1000,
            }),
            json!({
                "sessionId": "ses-main",
                "taskId": "task-explore",
                "toolCallId": "call_1",
                "agentType": "Explore",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "Found " },
                },
                "timestamp": 1001,
            }),
            json!({
                "sessionId": "ses-main",
                "taskId": "task-explore",
                "toolCallId": "call_1",
                "agentType": "Explore",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "entry point." },
                },
                "timestamp": 1002,
            }),
            json!({
                "sessionId": "ses-main",
                "taskId": "task-explore",
                "toolCallId": "call_1",
                "agentType": "Explore",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call_1",
                    "kind": "read",
                    "status": "pending",
                    "rawInput": { "path": "src/main.rs" },
                    "_meta": { "codeAgentCLI": { "toolName": "Read" } },
                },
                "timestamp": 1003,
            }),
            json!({
                "sessionId": "ses-main",
                "taskId": "task-explore",
                "toolCallId": "call_1",
                "agentType": "Explore",
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_1",
                    "status": "completed",
                    "rawOutput": "fn main() {}",
                },
                "timestamp": 1004,
            }),
        ];

        let mut records = Vec::new();
        for frame in frames {
            let translated =
                translate_extension_notification(subagent_ext(frame)).expect("valid notification");
            assert_eq!(translated.session_id.0.as_ref(), "ses-main");
            records.extend(assembler.push_update(&translated.update));
        }
        records.extend(assembler.end_turn(StopReason::EndTurn));

        let updates = records
            .into_iter()
            .filter_map(|record| match record.record {
                HistoryRecord::Update { update, .. } => Some(*update),
                _ => None,
            })
            .collect::<Vec<_>>();

        let expected_meta = json!({
            "_subagent/update": {
                "sessionId": "ses-main",
                "taskId": "task-explore",
                "toolCallId": "call_1",
                "agentType": "Explore",
            }
        });
        assert_eq!(
            updates,
            vec![
                serde_json::from_value::<SessionUpdate>(json!({
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "Checking files" },
                    "_meta": expected_meta,
                }))
                .unwrap(),
                serde_json::from_value::<SessionUpdate>(json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "Found entry point." },
                    "_meta": expected_meta,
                }))
                .unwrap(),
                // Inner `call_1` is scoped to `task-explore:call_1`, leaving parent `call_1` intact
                // and falling back to `_meta.codeAgentCLI.toolName` when `title` was omitted.
                serde_json::from_value::<SessionUpdate>(json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "task-explore:call_1",
                    "title": "Read",
                    "kind": "read",
                    "status": "completed",
                    "rawInput": { "path": "src/main.rs" },
                    "rawOutput": "fn main() {}",
                    "_meta": {
                        "_subagent/update": {
                            "sessionId": "ses-main",
                            "taskId": "task-explore",
                            "toolCallId": "call_1",
                            "agentType": "Explore",
                        },
                        "codeAgentCLI": { "toolName": "Read" },
                    },
                }))
                .unwrap(),
                SessionUpdate::ToolCall(
                    ToolCall::new("call_1", "Task: Explore").status(ToolCallStatus::Completed),
                ),
            ]
        );
        if let SessionUpdate::ToolCall(subagent_call) = &updates[2] {
            assert_eq!(
                child_source_id(subagent_call.meta.as_ref()),
                Some("task-explore")
            );
        } else {
            panic!("expected subagent tool call");
        }
    }

    #[test]
    fn subagent_tool_calls_with_colliding_raw_ids_do_not_unpause_parent_liveness() {
        const WINDOWS: [Duration; 1] = [Duration::from_secs(60)];
        let mut liveness = PromptLiveness::new(&WINDOWS);
        liveness.observe(&SessionUpdate::ToolCall(
            ToolCall::new("call_1", "Agent").status(ToolCallStatus::InProgress),
        ));
        assert_eq!(liveness.deadline(), None);

        let translated = translate_extension_notification(subagent_ext(json!({
            "sessionId": "ses-main",
            "taskId": "task-1",
            "toolCallId": "call_1",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_1",
                "title": "Bash",
                "status": "completed",
            },
            "timestamp": 1,
        })))
        .expect("translate subagent tool");
        liveness.observe(&translated.update);

        // Parent `call_1` must remain running and keep the inactivity deadline paused.
        assert_eq!(liveness.deadline(), None);
    }

    #[test]
    fn ignores_unknown_extensions_and_drops_invalid_or_session_mutating_subagent_frames() {
        assert_eq!(
            translate_extension_notification(ExtensionNotification {
                method: "_other/custom".to_string(),
                params: json!({}),
            }),
            None
        );

        for invalid in [
            json!(null),
            json!({"sessionId": "", "taskId": "t1", "update": {"sessionUpdate": "plan", "entries": []}}),
            json!({"sessionId": "s1", "taskId": " ", "update": {"sessionUpdate": "plan", "entries": []}}),
            json!({
                "sessionId": "s1",
                "taskId": "t1",
                "update": { "sessionUpdate": "session_info_update", "title": "Hijacked" },
            }),
            json!({
                "sessionId": "s1",
                "taskId": "t1",
                "update": { "sessionUpdate": "tool_call", "title": "Missing id" },
            }),
        ] {
            assert_eq!(
                translate_extension_notification(subagent_ext(invalid)),
                None
            );
        }
    }
}
