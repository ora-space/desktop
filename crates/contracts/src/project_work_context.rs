use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes which client surface owns one project work context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project-work-context.ts")]
pub enum ProjectWorkContextSurface {
    Web,
    Tauri,
}

/// Describes the public project work context payload shared across adapter responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project-work-context.ts")]
pub struct ProjectWorkContext {
    pub id: String,
    pub surface: ProjectWorkContextSurface,
    pub window_id: String,
    pub project_id: String,
    #[ts(type = "number")]
    pub lease_expires_at: i64,
}

/// Carries the payload used to open or switch a client window into a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project-work-context.ts")]
pub struct OpenProjectWorkContextRequest {
    pub surface: ProjectWorkContextSurface,
    pub window_id: String,
    pub project_id: String,
}

/// Returns the active project work context after a successful open or switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project-work-context.ts")]
pub struct OpenProjectWorkContextResponse {
    pub context: ProjectWorkContext,
}

/// Carries the payload used to renew an existing project work context lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project-work-context.ts")]
pub struct RenewProjectWorkContextRequest {
    pub surface: ProjectWorkContextSurface,
    pub window_id: String,
}

/// Returns the active project work context after a successful lease renewal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project-work-context.ts")]
pub struct RenewProjectWorkContextResponse {
    pub context: ProjectWorkContext,
}

#[cfg(test)]
mod tests {
    use super::{
        OpenProjectWorkContextRequest, OpenProjectWorkContextResponse, ProjectWorkContext,
        ProjectWorkContextSurface, RenewProjectWorkContextRequest, RenewProjectWorkContextResponse,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};

    /// Verifies project work context contracts serialize to frontend-friendly JSON payloads.
    #[test]
    fn serializes_project_work_context_contracts() {
        let context = ProjectWorkContext {
            id: "context-1".to_string(),
            surface: ProjectWorkContextSurface::Tauri,
            window_id: "window-1".to_string(),
            project_id: "project-1".to_string(),
            lease_expires_at: 120_000,
        };
        let open_request = OpenProjectWorkContextRequest {
            surface: ProjectWorkContextSurface::Web,
            window_id: "main".to_string(),
            project_id: "project-1".to_string(),
        };
        let renew_request = RenewProjectWorkContextRequest {
            surface: ProjectWorkContextSurface::Tauri,
            window_id: "window-1".to_string(),
        };

        assert_serialized_json(
            &context,
            json!({
                "id": "context-1",
                "surface": "tauri",
                "windowId": "window-1",
                "projectId": "project-1",
                "leaseExpiresAt": 120000,
            }),
        );
        assert_serialized_json(
            &open_request,
            json!({
                "surface": "web",
                "windowId": "main",
                "projectId": "project-1",
            }),
        );
        assert_serialized_json(
            &OpenProjectWorkContextResponse {
                context: context.clone(),
            },
            json!({
                "context": {
                    "id": "context-1",
                    "surface": "tauri",
                    "windowId": "window-1",
                    "projectId": "project-1",
                    "leaseExpiresAt": 120000,
                },
            }),
        );
        assert_serialized_json(
            &renew_request,
            json!({
                "surface": "tauri",
                "windowId": "window-1",
            }),
        );
        assert_serialized_json(
            &RenewProjectWorkContextResponse { context },
            json!({
                "context": {
                    "id": "context-1",
                    "surface": "tauri",
                    "windowId": "window-1",
                    "projectId": "project-1",
                    "leaseExpiresAt": 120000,
                },
            }),
        );
    }

    /// Verifies the shared public context shape stays stable across open and renew responses.
    #[test]
    fn preserves_shared_context_shape_across_responses() {
        let context = ProjectWorkContext {
            id: "context-1".to_string(),
            surface: ProjectWorkContextSurface::Web,
            window_id: "main".to_string(),
            project_id: "project-1".to_string(),
            lease_expires_at: 120_000,
        };

        assert_eq!(
            OpenProjectWorkContextResponse {
                context: context.clone(),
            }
            .context,
            RenewProjectWorkContextResponse {
                context: context.clone()
            }
            .context
        );
    }

    /// Serializes one value and compares it against the expected JSON shape.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        let actual = serde_json::to_value(value)
            .unwrap_or_else(|error| panic!("expected serialization to succeed: {error}"));

        assert_eq!(actual, expected);
    }
}
