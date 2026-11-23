use super::{current_model, produced_output, stale_load_retry_allowed};
use crate::attach::LoadRestoration;
use agent_client_protocol_schema::v1::{
    ContentBlock, ContentChunk, Plan, SessionUpdate, TextContent, ToolCall, ToolCallUpdate,
    ToolCallUpdateFields,
};
use agent_client_protocol_schema::v1::{
    SessionConfigId, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelect, SessionConfigSelectOption, SessionConfigSelectOptions,
    SessionConfigValueId,
};

#[test]
fn a_stale_load_retry_requires_no_output_and_only_one_attempt() {
    assert!(stale_load_retry_allowed(
        true,
        false,
        true,
        LoadRestoration::Restored
    ));
    assert!(!stale_load_retry_allowed(
        true,
        true,
        true,
        LoadRestoration::Restored
    ));
    assert!(!stale_load_retry_allowed(
        true,
        false,
        false,
        LoadRestoration::Restored
    ));
    assert!(!stale_load_retry_allowed(
        false,
        false,
        true,
        LoadRestoration::Restored
    ));
}

#[test]
fn output_classification_covers_text_tools_and_controls() {
    let text = || {
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            "hello",
        ))))
    };
    for update in [
        text(),
        SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            "thinking",
        )))),
        SessionUpdate::ToolCall(ToolCall::new("tool", "Tool")),
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("tool", ToolCallUpdateFields::new())),
    ] {
        assert!(produced_output(&update), "{update:?}");
    }
    // A plan can fail without any assistant output or tool side effect and is retryable.
    assert!(!produced_output(&SessionUpdate::Plan(
        Plan::new(Vec::new())
    )));
}

/// An empty configuration report carries no model and must not terminate the actor.
#[test]
fn an_empty_provider_report_has_no_rebuild_model() {
    pretty_assertions::assert_eq!(current_model(&[]), None);
}

#[test]
fn the_rebuild_model_comes_from_the_restored_provider() {
    let option = SessionConfigOption::new(
        SessionConfigId::new("model"),
        "Model",
        SessionConfigKind::Select(SessionConfigSelect::new(
            SessionConfigValueId::new("deepseek/deepseek-v4-pro"),
            SessionConfigSelectOptions::Ungrouped(vec![SessionConfigSelectOption::new(
                "deepseek/deepseek-v4-pro",
                "DeepSeek V4 Pro",
            )]),
        )),
    )
    .category(SessionConfigOptionCategory::Model);
    assert_eq!(
        current_model(&[option]),
        Some("deepseek/deepseek-v4-pro".to_string())
    );
}
