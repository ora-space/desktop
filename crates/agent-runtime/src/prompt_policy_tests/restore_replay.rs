//! Exercises the provider's load response as the boundary between replay and live updates.

use super::{AgentInput, Fixture, prompt, run_case, s1_subagent_frame};
use agent_client_protocol_schema::v1::{SessionUpdate, StopReason};
use ora_contracts::PromptSessionEvent;
use ora_history::{HistoryLine, HistoryRecord, history_path};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const SESSION: &str = "sess-1";

/// The same two wire shapes on either side of the load response make invalid fixtures observable.
fn child_frames(phase: &str) -> [Value; 2] {
    [
        s1_subagent_frame(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": format!("{phase} child message")},
        })),
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": SESSION,
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": format!("{phase}-tool"),
                    "title": "Read",
                    "status": "completed",
                    "_meta": {"_subagent/update": {"sessionId": SESSION, "taskId": "task-1"}},
                },
            },
        }),
    ]
}

/// Provider replay stays out of the prompt stream and physical JSONL, while live child frames
/// on the same restored channel reach both destinations with their source intact.
#[test]
fn session_load_isolates_subagent_replay_from_live_updates() {
    run_case(async {
        let mut fixture = Fixture::with_load_support(/*load_session_supported*/ true).await;
        fixture.seed_session(SESSION);
        let manager = fixture.manager.clone();
        let admission = tokio::spawn(async move { manager.prompt_session(prompt(SESSION)).await });

        let load = fixture.next_frame().await;
        assert_eq!(load["method"], "session/load");
        assert_eq!(load["params"]["sessionId"], SESSION);
        for frame in child_frames("replay") {
            fixture.agent.send(AgentInput::RawFrame(frame)).unwrap();
        }
        fixture
            .agent
            .send(AgentInput::RawFrame(json!({
                "jsonrpc": "2.0", "id": load["id"], "result": {},
            })))
            .unwrap();

        let mut stream = admission.await.unwrap().unwrap();
        let sent = fixture.next_frame().await;
        assert_eq!(sent["method"], "session/prompt");
        assert_eq!(sent["params"]["sessionId"], SESSION);
        let [extension, ordinary] = child_frames("live");
        let mut live_chunk = extension["params"]["update"].clone();
        live_chunk["_meta"] = json!({"_subagent/update": {
            "sessionId": SESSION, "taskId": "task-1", "toolCallId": "call-1", "agentType": "Explore",
        }});
        let expected: Vec<SessionUpdate> = [live_chunk, ordinary["params"]["update"].clone()]
            .into_iter()
            .map(|value| serde_json::from_value(value).unwrap())
            .collect();
        for frame in [extension, ordinary] {
            fixture.agent.send(AgentInput::RawFrame(frame)).unwrap();
        }
        fixture
            .agent
            .send(AgentInput::Finish(SESSION.to_owned(), StopReason::EndTurn))
            .unwrap();

        let mut forwarded = Vec::new();
        let mut completions = Vec::new();
        while let Some(event) = stream.recv().await {
            match event.unwrap() {
                // The restored session's options lead the stream; they are not part of the turn.
                PromptSessionEvent::SessionUpdate {
                    update: SessionUpdate::ConfigOptionUpdate(_),
                    ..
                } => {}
                PromptSessionEvent::SessionUpdate { update, .. } => forwarded.push(update),
                PromptSessionEvent::Completed { stop_reason, .. } => completions.push(stop_reason),
                _ => {}
            }
        }
        assert_eq!(completions, [StopReason::EndTurn]);

        // Read every physical line: the normal history reader can coalesce duplicate positions.
        let path = history_path(&fixture.manager.inner.sessions_root, SESSION).unwrap();
        let jsonl = std::fs::read_to_string(path).unwrap();
        let mut recorded = Vec::new();
        for line in jsonl.lines() {
            if let HistoryRecord::Update { update, .. } =
                serde_json::from_str::<HistoryLine>(line).unwrap().record
                && !matches!(*update, SessionUpdate::UserMessageChunk(_))
            {
                recorded.push(*update);
            }
        }
        assert_eq!((&forwarded, &recorded), (&expected, &expected));
        fixture.shutdown().await;
    });
}
