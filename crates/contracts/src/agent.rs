use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes a public configurable-agent payload without persistence audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Carries the public fields required to create a configurable agent type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct CreateAgentRequest {
    pub name: String,
    pub description: String,
}

/// Returns one created configurable agent type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct CreateAgentResponse {
    pub agent: Agent,
}

/// Identifies the visible configurable agent type requested by identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct GetAgentRequest {
    pub agent_id: String,
}

/// Returns one visible configurable agent type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct GetAgentResponse {
    pub agent: Agent,
}

/// Requests every visible configurable agent type in stable storage order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct ListAgentsRequest {}

/// Returns every visible configurable agent type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct ListAgentsResponse {
    pub agents: Vec<Agent>,
}

/// Replaces one configurable agent type located by its stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct UpdateAgentRequest {
    pub agent_id: String,
    pub name: String,
    pub description: String,
}

/// Returns the replacement configurable agent type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct UpdateAgentResponse {
    pub agent: Agent,
}

/// Identifies the visible configurable agent type to soft-delete by identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct DeleteAgentRequest {
    pub agent_id: String,
}

/// Returns the identifier of the configurable agent type that was soft-deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent.ts")]
pub struct DeleteAgentResponse {
    pub agent_id: String,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    Agent::export(config)?;
    CreateAgentRequest::export(config)?;
    CreateAgentResponse::export(config)?;
    GetAgentRequest::export(config)?;
    GetAgentResponse::export(config)?;
    ListAgentsRequest::export(config)?;
    ListAgentsResponse::export(config)?;
    UpdateAgentRequest::export(config)?;
    UpdateAgentResponse::export(config)?;
    DeleteAgentRequest::export(config)?;
    DeleteAgentResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Agent, CreateAgentRequest, CreateAgentResponse, DeleteAgentRequest, DeleteAgentResponse,
        GetAgentRequest, GetAgentResponse, ListAgentsRequest, ListAgentsResponse,
        UpdateAgentRequest, UpdateAgentResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies public agent payloads exclude persistence-owned audit fields.
    #[test]
    fn serializes_agent_contract_without_audit_fields() {
        let agent = Agent {
            id: "agent-1".to_string(),
            name: "opencode".to_string(),
            description: "OpenCode agent configuration".to_string(),
        };

        assert_eq!(
            serde_json::to_value(agent).unwrap(),
            json!({
                "id": "agent-1",
                "name": "opencode",
                "description": "OpenCode agent configuration",
            })
        );
    }

    /// Verifies agent CRUD requests preserve the resource identifier separately from editable fields.
    #[test]
    fn serializes_agent_crud_contracts() {
        let agent = Agent {
            id: "agent-1".to_string(),
            name: "opencode".to_string(),
            description: "OpenCode agent configuration".to_string(),
        };

        assert_serialized_json(
            &CreateAgentRequest {
                name: agent.name.clone(),
                description: agent.description.clone(),
            },
            json!({ "name": "opencode", "description": "OpenCode agent configuration" }),
        );
        assert_serialized_json(
            &CreateAgentResponse {
                agent: agent.clone(),
            },
            json!({ "agent": { "id": "agent-1", "name": "opencode", "description": "OpenCode agent configuration" } }),
        );
        assert_serialized_json(
            &GetAgentRequest {
                agent_id: "agent-1".to_string(),
            },
            json!({ "agentId": "agent-1" }),
        );
        assert_serialized_json(
            &GetAgentResponse {
                agent: agent.clone(),
            },
            json!({ "agent": { "id": "agent-1", "name": "opencode", "description": "OpenCode agent configuration" } }),
        );
        assert_serialized_json(&ListAgentsRequest {}, json!({}));
        assert_serialized_json(
            &ListAgentsResponse {
                agents: vec![agent.clone()],
            },
            json!({ "agents": [{ "id": "agent-1", "name": "opencode", "description": "OpenCode agent configuration" }] }),
        );
        assert_serialized_json(
            &UpdateAgentRequest {
                agent_id: "agent-1".to_string(),
                name: "reviewer".to_string(),
                description: "Reviews changes".to_string(),
            },
            json!({ "agentId": "agent-1", "name": "reviewer", "description": "Reviews changes" }),
        );
        assert_serialized_json(
            &UpdateAgentResponse {
                agent: Agent {
                    id: "agent-1".to_string(),
                    name: "reviewer".to_string(),
                    description: "Reviews changes".to_string(),
                },
            },
            json!({ "agent": { "id": "agent-1", "name": "reviewer", "description": "Reviews changes" } }),
        );
        assert_serialized_json(
            &DeleteAgentRequest {
                agent_id: "agent-1".to_string(),
            },
            json!({ "agentId": "agent-1" }),
        );
        assert_serialized_json(
            &DeleteAgentResponse {
                agent_id: "agent-1".to_string(),
            },
            json!({ "agentId": "agent-1" }),
        );
    }

    fn assert_serialized_json(value: &impl serde::Serialize, expected: serde_json::Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
