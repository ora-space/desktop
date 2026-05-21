use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes whether the public session view is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub enum SessionStatus {
    Running,
    Stopped,
}

/// Describes the initial PTY dimensions used only during terminal session startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct TerminalSessionStartup {
    pub cols: u16,
    pub rows: u16,
}

/// Describes the public session payload shared across adapter responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct Session {
    pub id: String,
    pub task_id: String,
    pub agent_id: String,
    pub agent_session_id: Option<String>,
    pub status: SessionStatus,
}

/// Carries the app-facing payload for session creation requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct CreateSessionRequest {
    pub task_id: String,
    pub agent_id: String,
    pub agent_session_id: Option<String>,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalSessionStartup>,
}

/// Returns the created session after a successful create request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct CreateSessionResponse {
    pub session: Session,
}

/// Identifies which session to fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct GetSessionRequest {
    pub session_id: String,
}

/// Returns one session payload after a successful fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct GetSessionResponse {
    pub session: Session,
}

/// Requests the full visible session list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct ListSessionsRequest {}

/// Returns the visible session list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct ListSessionsResponse {
    pub sessions: Vec<Session>,
}

/// Carries the full replacement payload for session updates in the first slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct UpdateSessionRequest {
    pub session_id: String,
    pub task_id: String,
    pub agent_id: String,
    pub agent_session_id: Option<String>,
    pub status: SessionStatus,
}

/// Returns the updated session after a successful update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct UpdateSessionResponse {
    pub session: Session,
}

/// Identifies which session to delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct DeleteSessionRequest {
    pub session_id: String,
}

/// Returns the deleted session identifier after a successful delete request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct DeleteSessionResponse {
    pub session_id: String,
}

/// Describes one client-to-server terminal control message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub enum TerminalClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Kill {},
}

/// Describes one server-to-client terminal stream message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub enum TerminalServerMessage {
    Ready {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    History {
        data: String,
    },
    Output {
        data: String,
    },
    Exit {
        #[serde(rename = "exitCode")]
        exit_code: Option<i32>,
    },
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
        GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse, Session,
        SessionStatus, TerminalClientMessage, TerminalServerMessage, TerminalSessionStartup,
        UpdateSessionRequest, UpdateSessionResponse,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};

    /// Verifies the first session slice serializes to frontend-friendly JSON payloads.
    #[test]
    fn serializes_session_contracts() {
        let session = Session {
            id: "session-1".to_string(),
            task_id: "task-1".to_string(),
            agent_id: "agent-1".to_string(),
            agent_session_id: Some("provider-1".to_string()),
            status: SessionStatus::Running,
        };
        let create_request = CreateSessionRequest {
            task_id: "task-1".to_string(),
            agent_id: "agent-1".to_string(),
            agent_session_id: None,
            status: SessionStatus::Stopped,
            terminal: Some(TerminalSessionStartup { cols: 80, rows: 24 }),
        };
        let get_request = GetSessionRequest {
            session_id: "session-1".to_string(),
        };
        let list_request = ListSessionsRequest {};
        let update_request = UpdateSessionRequest {
            session_id: "session-1".to_string(),
            task_id: "task-2".to_string(),
            agent_id: "agent-2".to_string(),
            agent_session_id: Some("provider-2".to_string()),
            status: SessionStatus::Stopped,
        };
        let delete_request = DeleteSessionRequest {
            session_id: "session-1".to_string(),
        };

        assert_serialized_json(
            &session,
            json!({
                "id": "session-1",
                "taskId": "task-1",
                "agentId": "agent-1",
                "agentSessionId": "provider-1",
                "status": "running",
            }),
        );
        assert_serialized_json(
            &create_request,
            json!({
                "taskId": "task-1",
                "agentId": "agent-1",
                "agentSessionId": null,
                "status": "stopped",
                "terminal": {
                    "cols": 80,
                    "rows": 24,
                },
            }),
        );
        assert_serialized_json(
            &CreateSessionResponse {
                session: session.clone(),
            },
            json!({
                "session": {
                    "id": "session-1",
                    "taskId": "task-1",
                    "agentId": "agent-1",
                    "agentSessionId": "provider-1",
                    "status": "running",
                },
            }),
        );
        assert_serialized_json(&get_request, json!({ "sessionId": "session-1" }));
        assert_serialized_json(
            &GetSessionResponse {
                session: session.clone(),
            },
            json!({
                "session": {
                    "id": "session-1",
                    "taskId": "task-1",
                    "agentId": "agent-1",
                    "agentSessionId": "provider-1",
                    "status": "running",
                },
            }),
        );
        assert_serialized_json(&list_request, json!({}));
        assert_serialized_json(
            &ListSessionsResponse {
                sessions: vec![session.clone()],
            },
            json!({
                "sessions": [
                    {
                        "id": "session-1",
                        "taskId": "task-1",
                        "agentId": "agent-1",
                        "agentSessionId": "provider-1",
                        "status": "running",
                    },
                ],
            }),
        );
        assert_serialized_json(
            &update_request,
            json!({
                "sessionId": "session-1",
                "taskId": "task-2",
                "agentId": "agent-2",
                "agentSessionId": "provider-2",
                "status": "stopped",
            }),
        );
        assert_serialized_json(
            &UpdateSessionResponse { session },
            json!({
                "session": {
                    "id": "session-1",
                    "taskId": "task-1",
                    "agentId": "agent-1",
                    "agentSessionId": "provider-1",
                    "status": "running",
                },
            }),
        );
        assert_serialized_json(&delete_request, json!({ "sessionId": "session-1" }));
        assert_serialized_json(
            &DeleteSessionResponse {
                session_id: "session-1".to_string(),
            },
            json!({ "sessionId": "session-1" }),
        );
    }

    /// Confirms the shared session view remains the single reusable payload across responses.
    #[test]
    fn preserves_shared_session_shape_across_responses() {
        let session = Session {
            id: "session-1".to_string(),
            task_id: "task-1".to_string(),
            agent_id: "agent-1".to_string(),
            agent_session_id: None,
            status: SessionStatus::Stopped,
        };

        assert_eq!(
            CreateSessionResponse {
                session: session.clone(),
            },
            CreateSessionResponse {
                session: session.clone(),
            }
        );
        assert_eq!(
            GetSessionResponse {
                session: session.clone(),
            },
            GetSessionResponse {
                session: session.clone(),
            }
        );
        assert_eq!(
            ListSessionsResponse {
                sessions: vec![session.clone()],
            },
            ListSessionsResponse {
                sessions: vec![session.clone()],
            }
        );
        assert_eq!(
            UpdateSessionResponse {
                session: session.clone(),
            },
            UpdateSessionResponse { session }
        );
    }

    /// Verifies terminal session startup payloads and stream messages serialize with stable JSON shapes.
    #[test]
    fn serializes_terminal_contracts() {
        assert_serialized_json(
            &TerminalSessionStartup {
                cols: 120,
                rows: 40,
            },
            json!({
                "cols": 120,
                "rows": 40,
            }),
        );
        assert_serialized_json(
            &TerminalClientMessage::Input {
                data: "ls -la\n".to_string(),
            },
            json!({
                "type": "input",
                "data": "ls -la\n",
            }),
        );
        assert_serialized_json(
            &TerminalClientMessage::Resize {
                cols: 132,
                rows: 50,
            },
            json!({
                "type": "resize",
                "cols": 132,
                "rows": 50,
            }),
        );
        assert_serialized_json(
            &TerminalClientMessage::Kill {},
            json!({
                "type": "kill",
            }),
        );
        assert_serialized_json(
            &TerminalServerMessage::Ready {
                session_id: "session-1".to_string(),
            },
            json!({
                "type": "ready",
                "sessionId": "session-1",
            }),
        );
        assert_serialized_json(
            &TerminalServerMessage::History {
                data: "cargo test\n".to_string(),
            },
            json!({
                "type": "history",
                "data": "cargo test\n",
            }),
        );
        assert_serialized_json(
            &TerminalServerMessage::Output {
                data: "Finished dev profile\n".to_string(),
            },
            json!({
                "type": "output",
                "data": "Finished dev profile\n",
            }),
        );
        assert_serialized_json(
            &TerminalServerMessage::Exit { exit_code: Some(0) },
            json!({
                "type": "exit",
                "exitCode": 0,
            }),
        );
        assert_serialized_json(
            &TerminalServerMessage::Error {
                code: "terminal_already_attached".to_string(),
                message: "terminal already attached".to_string(),
            },
            json!({
                "type": "error",
                "code": "terminal_already_attached",
                "message": "terminal already attached",
            }),
        );
    }

    /// Verifies terminal startup dimensions remain separate from later resize control messages.
    #[test]
    fn keeps_terminal_startup_dimensions_separate_from_resize_messages() {
        let create_request = CreateSessionRequest {
            task_id: "task-1".to_string(),
            agent_id: "terminal".to_string(),
            agent_session_id: None,
            status: SessionStatus::Running,
            terminal: Some(TerminalSessionStartup { cols: 90, rows: 28 }),
        };
        let resize_message = TerminalClientMessage::Resize {
            cols: 140,
            rows: 45,
        };

        assert_eq!(
            serde_json::to_value(create_request).unwrap(),
            json!({
                "taskId": "task-1",
                "agentId": "terminal",
                "agentSessionId": null,
                "status": "running",
                "terminal": {
                    "cols": 90,
                    "rows": 28,
                },
            })
        );
        assert_eq!(
            serde_json::to_value(resize_message).unwrap(),
            json!({
                "type": "resize",
                "cols": 140,
                "rows": 45,
            })
        );
    }

    /// Serializes one value and compares the full JSON payload so field names stay stable.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
