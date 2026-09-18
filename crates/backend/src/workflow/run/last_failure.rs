use ora_application::NodeFailureKind;
use ora_domain::WorkflowNodeRun;
use serde::Deserialize;

const OUTPUT_CHAR_LIMIT: usize = 4000;

/// The previous failed attempt of the node being dispatched, already filtered for injection.
pub(crate) struct PreviousFailure {
    pub(crate) attempt: u32,
    pub(crate) kind: NodeFailureKind,
    pub(crate) message: String,
    pub(crate) output: Option<String>,
}

#[derive(Deserialize)]
struct PayloadView {
    error_detail: Option<ErrorDetailView>,
}

#[derive(Deserialize)]
struct ErrorDetailView {
    kind: NodeFailureKind,
    #[serde(default)]
    message: String,
    attempt: u32,
}

/// `None` unless the switch is on, a previous failed attempt exists, its `payload.error_detail`
/// parses, and `kind.inject_into_prompt()`. `output` is kept only for `StructuredOutput`
/// (the agent must see what it produced), truncated to 4000 chars with a trailing `…` marker.
pub(crate) fn previous_failure_for_injection(
    inject: bool,
    previous: Option<&WorkflowNodeRun>,
) -> Option<PreviousFailure> {
    if !inject {
        return None;
    }
    let previous = previous?;
    let detail = parse_error_detail(previous.payload.as_deref())?;
    if !detail.kind.inject_into_prompt() {
        return None;
    }
    let message = if detail.message.is_empty() {
        previous.error.clone().unwrap_or_default()
    } else {
        detail.message
    };
    let output = if detail.kind == NodeFailureKind::StructuredOutput {
        previous.output.as_deref().map(truncate_output)
    } else {
        None
    };
    Some(PreviousFailure {
        attempt: detail.attempt,
        kind: detail.kind,
        message,
        output,
    })
}

/// Reads `error_detail` the way rollback parses node payloads: unparsable blobs become `None`.
fn parse_error_detail(payload: Option<&str>) -> Option<ErrorDetailView> {
    let payload = payload?;
    serde_json::from_str::<PayloadView>(payload)
        .ok()?
        .error_detail
}

/// Keeps the first 4000 characters and marks overflow so the prompt stays bounded.
fn truncate_output(output: &str) -> String {
    let mut chars = output.chars();
    let truncated: String = chars.by_ref().take(OUTPUT_CHAR_LIMIT).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::{OUTPUT_CHAR_LIMIT, previous_failure_for_injection};
    use ora_application::NodeFailureKind;
    use ora_domain::{
        AuditFields, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId,
    };
    use pretty_assertions::assert_eq;

    /// Builds a soft-deleted failed node whose payload carries a typed `error_detail`.
    fn failed_node(
        kind: NodeFailureKind,
        message: &str,
        attempt: u32,
        output: Option<&str>,
    ) -> WorkflowNodeRun {
        let payload = serde_json::json!({
            "error_detail": {
                "kind": kind,
                "message": message,
                "attempt": attempt,
            }
        })
        .to_string();
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new("nr-1"),
            WorkflowRunId::new("run-1"),
            ora_domain::WorkflowScopeId::new("root:test"),
            "review",
            "agent",
            /*session_id*/ None,
            WorkflowNodeStatus::Failed,
            /*input*/ None,
            output.map(str::to_string),
            /*error*/ None,
            Some(payload),
            /*started_at*/ Some(1),
            /*finished_at*/ Some(2),
            AuditFields::new(1, 2, /*is_deleted*/ true),
        )
    }

    #[test]
    fn previous_failure_for_injection_returns_none_without_previous() {
        assert!(previous_failure_for_injection(true, None).is_none());
    }

    #[test]
    fn previous_failure_for_injection_returns_none_when_switch_off() {
        let previous = failed_node(
            NodeFailureKind::StructuredOutput,
            "bad json",
            2,
            Some("{bad json"),
        );
        assert!(previous_failure_for_injection(false, Some(&previous)).is_none());
    }

    #[test]
    fn previous_failure_for_injection_returns_none_for_session_failure() {
        let previous = failed_node(NodeFailureKind::Session, "agent died", 1, None);
        assert!(previous_failure_for_injection(true, Some(&previous)).is_none());
    }

    #[test]
    fn previous_failure_for_injection_keeps_structured_output() {
        let previous = failed_node(
            NodeFailureKind::StructuredOutput,
            "bad json",
            2,
            Some("{bad json"),
        );
        let injected = previous_failure_for_injection(true, Some(&previous)).unwrap();
        assert_eq!(injected.attempt, 2);
        assert_eq!(injected.kind, NodeFailureKind::StructuredOutput);
        assert_eq!(injected.message, "bad json");
        assert_eq!(injected.output.as_deref(), Some("{bad json"));
    }

    #[test]
    fn previous_failure_for_injection_drops_output_for_agent_refusal() {
        let previous = failed_node(
            NodeFailureKind::AgentRefusal,
            "agent refused the request",
            1,
            Some("I will not do that"),
        );
        let injected = previous_failure_for_injection(true, Some(&previous)).unwrap();
        assert_eq!(injected.kind, NodeFailureKind::AgentRefusal);
        assert_eq!(injected.output, None);
    }

    #[test]
    fn previous_failure_for_injection_truncates_long_output() {
        let output = "x".repeat(OUTPUT_CHAR_LIMIT + 1);
        let previous = failed_node(
            NodeFailureKind::StructuredOutput,
            "too long",
            3,
            Some(&output),
        );
        let injected = previous_failure_for_injection(true, Some(&previous)).unwrap();
        let expected = format!("{}…", "x".repeat(OUTPUT_CHAR_LIMIT));
        assert_eq!(injected.output.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn previous_failure_for_injection_falls_back_to_node_error_when_detail_message_is_empty() {
        let mut previous = failed_node(NodeFailureKind::AgentRefusal, "", 1, None);
        previous.error = Some("refused".to_string());
        let injected = previous_failure_for_injection(true, Some(&previous)).unwrap();
        assert_eq!(injected.message, "refused");
    }

    /// Unparsable payloads must not invent a failure block for the next attempt.
    #[test]
    fn previous_failure_for_injection_returns_none_for_unparsable_payload() {
        let previous = WorkflowNodeRun::new(
            WorkflowNodeRunId::new("nr-1"),
            WorkflowRunId::new("run-1"),
            ora_domain::WorkflowScopeId::new("root:test"),
            "review",
            "agent",
            /*session_id*/ None,
            WorkflowNodeStatus::Failed,
            /*input*/ None,
            /*output*/ None,
            /*error*/ Some("broken".to_string()),
            Some("not-json".to_string()),
            /*started_at*/ Some(1),
            /*finished_at*/ Some(2),
            AuditFields::new(1, 2, /*is_deleted*/ true),
        );
        assert!(previous_failure_for_injection(true, Some(&previous)).is_none());
    }
}
