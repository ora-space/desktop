use pretty_assertions::assert_eq;
use serde_json::json;

use super::{
    BlobResourceContents, ContentBlock, ContentToolCallContent, EmbeddedResourceContents,
    ImageContent, ResourceLink, SessionPromptRequest, SessionPromptResponse,
    SessionRequestPermissionResponse, SessionUpdate, SessionUpdateNotification, StopReason,
    TextContent, TextResourceContents, ToolCall, ToolCallContent, ToolCallStatus, ToolKind,
    ToolPermissionOutcome,
};

/// Ensures all documented content variants preserve their tagged wire representation.
#[test]
fn round_trips_content_blocks() {
    let blocks = vec![
        ContentBlock::Text(TextContent {
            text: "hello".to_owned(),
            annotations: None,
        }),
        ContentBlock::Image(ImageContent {
            data: "base64".to_owned(),
            mime_type: "image/png".to_owned(),
            uri: None,
            annotations: None,
        }),
        ContentBlock::Resource {
            resource: EmbeddedResourceContents::Text(TextResourceContents {
                uri: "file:///main.rs".to_owned(),
                text: "fn main() {}".to_owned(),
                mime_type: Some("text/rust".to_owned()),
            }),
            annotations: None,
        },
        ContentBlock::Resource {
            resource: EmbeddedResourceContents::Blob(BlobResourceContents {
                uri: "file:///asset.bin".to_owned(),
                blob: "base64".to_owned(),
                mime_type: None,
            }),
            annotations: None,
        },
        ContentBlock::ResourceLink(ResourceLink {
            uri: "file:///guide.pdf".to_owned(),
            name: "guide.pdf".to_owned(),
            mime_type: Some("application/pdf".to_owned()),
            title: None,
            description: None,
            size: Some(1024),
            annotations: None,
        }),
    ];

    for block in blocks {
        let serialized = serde_json::to_value(&block)
            .unwrap_or_else(|error| panic!("content should serialize: {error}"));
        let deserialized = serde_json::from_value(serialized)
            .unwrap_or_else(|error| panic!("content should deserialize: {error}"));

        assert_eq!(block, deserialized);
    }
}

/// Ensures request fields use ACP camelCase names without including a JSON-RPC envelope.
#[test]
fn serializes_session_prompt_params_only() {
    let request = SessionPromptRequest {
        session_id: "sess_123".to_owned(),
        message_id: Some("4c12d49b-729c-4086-bfed-5b82e9a53400".to_owned()),
        prompt: vec![ContentBlock::Text(TextContent {
            text: "hello".to_owned(),
            annotations: None,
        })],
    };

    assert_eq!(
        json!({
            "sessionId": "sess_123",
            "messageId": "4c12d49b-729c-4086-bfed-5b82e9a53400",
            "prompt": [{ "type": "text", "text": "hello" }]
        }),
        serde_json::to_value(request)
            .unwrap_or_else(|error| panic!("prompt request should serialize: {error}"))
    );
}

/// Ensures prompt responses acknowledge the user message identifier when supported.
#[test]
fn serializes_prompt_response_message_acknowledgement() {
    let response = SessionPromptResponse {
        stop_reason: StopReason::EndTurn,
        user_message_id: Some("4c12d49b-729c-4086-bfed-5b82e9a53400".to_owned()),
    };

    assert_eq!(
        json!({
            "stopReason": "end_turn",
            "userMessageId": "4c12d49b-729c-4086-bfed-5b82e9a53400"
        }),
        serde_json::to_value(response)
            .unwrap_or_else(|error| panic!("prompt response should serialize: {error}"))
    );
}

/// Ensures omitted tool presentation fields receive the defaults defined by ACP.
#[test]
fn applies_tool_call_defaults() {
    let tool_call: ToolCall = serde_json::from_value(json!({
        "toolCallId": "call_123",
        "title": "Read file"
    }))
    .unwrap_or_else(|error| panic!("minimal tool call should deserialize: {error}"));

    assert_eq!(
        ToolCall {
            tool_call_id: "call_123".to_owned(),
            title: "Read file".to_owned(),
            kind: ToolKind::Other,
            status: ToolCallStatus::Pending,
            content: None,
            locations: None,
            raw_input: None,
            raw_output: None,
        },
        tool_call
    );
}

/// Ensures nested session updates and permission outcomes use their protocol discriminators.
#[test]
fn serializes_tagged_updates_and_permission_outcomes() {
    let notification = SessionUpdateNotification {
        session_id: "sess_123".to_owned(),
        update: SessionUpdate::ToolCall(ToolCall {
            tool_call_id: "call_123".to_owned(),
            title: "Analyze".to_owned(),
            kind: ToolKind::Other,
            status: ToolCallStatus::Pending,
            content: Some(vec![ToolCallContent::Content(ContentToolCallContent {
                content: ContentBlock::Text(TextContent {
                    text: "working".to_owned(),
                    annotations: None,
                }),
            })]),
            locations: None,
            raw_input: None,
            raw_output: None,
        }),
    };
    let permission = SessionRequestPermissionResponse {
        outcome: ToolPermissionOutcome::Selected {
            option_id: "allow-once".to_owned(),
        },
    };

    assert_eq!(
        json!({
            "sessionId": "sess_123",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_123",
                "title": "Analyze",
                "kind": "other",
                "status": "pending",
                "content": [{
                    "type": "content",
                    "content": { "type": "text", "text": "working" }
                }]
            }
        }),
        serde_json::to_value(notification)
            .unwrap_or_else(|error| panic!("session update should serialize: {error}"))
    );
    assert_eq!(
        json!({
            "outcome": {
                "outcome": "selected",
                "optionId": "allow-once"
            }
        }),
        serde_json::to_value(permission)
            .unwrap_or_else(|error| panic!("permission response should serialize: {error}"))
    );
}
