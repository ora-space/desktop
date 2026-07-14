use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes whether the public session view is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub enum SessionStatus {
    Running,
    Stopped,
}

/// Describes the public session payload shared across adapter responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
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
#[ts(export_to = "session.ts")]
pub struct CreateSessionRequest {
    pub task_id: String,
    pub agent_id: String,
    pub agent_session_id: Option<String>,
    pub status: SessionStatus,
}

/// Returns the created session after a successful create request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct CreateSessionResponse {
    pub session: Session,
}

/// Identifies which session to fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct GetSessionRequest {
    pub session_id: String,
}

/// Returns one session payload after a successful fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct GetSessionResponse {
    pub session: Session,
}

/// Requests the full visible session list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct ListSessionsRequest {}

/// Returns the visible session list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct ListSessionsResponse {
    pub sessions: Vec<Session>,
}

/// Carries the full replacement payload for session updates in the first slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
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
#[ts(export_to = "session.ts")]
pub struct UpdateSessionResponse {
    pub session: Session,
}

/// Identifies which session to delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct DeleteSessionRequest {
    pub session_id: String,
}

/// Returns the deleted session identifier after a successful delete request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct DeleteSessionResponse {
    pub session_id: String,
}

#[cfg(test)]
mod tests {
    use super::{
        CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
        GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse, Session,
        SessionStatus, UpdateSessionRequest, UpdateSessionResponse,
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

    /// Serializes one value and compares the full JSON payload so field names stay stable.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
