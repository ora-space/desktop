use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes a public skill payload without persistence audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Carries the public fields required to create a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct CreateSkillRequest {
    pub name: String,
    pub description: String,
}

/// Returns one created skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct CreateSkillResponse {
    pub skill: Skill,
}

/// Identifies the visible skill requested by identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct GetSkillRequest {
    pub skill_id: String,
}

/// Returns one visible skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct GetSkillResponse {
    pub skill: Skill,
}

/// Requests every visible skill in stable storage order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct ListSkillsRequest {}

/// Returns every visible skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct ListSkillsResponse {
    pub skills: Vec<Skill>,
}

/// Replaces one skill located by its stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct UpdateSkillRequest {
    pub skill_id: String,
    pub name: String,
    pub description: String,
}

/// Returns the replacement skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct UpdateSkillResponse {
    pub skill: Skill,
}

/// Identifies the visible skill to soft-delete by identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct DeleteSkillRequest {
    pub skill_id: String,
}

/// Returns the identifier of the skill that was soft-deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct DeleteSkillResponse {
    pub skill_id: String,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    Skill::export(config)?;
    CreateSkillRequest::export(config)?;
    CreateSkillResponse::export(config)?;
    GetSkillRequest::export(config)?;
    GetSkillResponse::export(config)?;
    ListSkillsRequest::export(config)?;
    ListSkillsResponse::export(config)?;
    UpdateSkillRequest::export(config)?;
    UpdateSkillResponse::export(config)?;
    DeleteSkillRequest::export(config)?;
    DeleteSkillResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
        GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, Skill,
        UpdateSkillRequest, UpdateSkillResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies public skill payloads exclude persistence-owned audit fields.
    #[test]
    fn serializes_skill_contract_without_audit_fields() {
        let skill = Skill {
            id: "skill-1".to_string(),
            name: "review".to_string(),
            description: "Reviews implementation changes".to_string(),
        };

        assert_eq!(
            serde_json::to_value(skill).unwrap(),
            json!({
                "id": "skill-1",
                "name": "review",
                "description": "Reviews implementation changes",
            })
        );
    }

    /// Verifies skill CRUD requests preserve the resource identifier separately from editable fields.
    #[test]
    fn serializes_skill_crud_contracts() {
        let skill = Skill {
            id: "skill-1".to_string(),
            name: "review".to_string(),
            description: "Reviews implementation changes".to_string(),
        };

        assert_serialized_json(
            &CreateSkillRequest {
                name: skill.name.clone(),
                description: skill.description.clone(),
            },
            json!({ "name": "review", "description": "Reviews implementation changes" }),
        );
        assert_serialized_json(
            &CreateSkillResponse {
                skill: skill.clone(),
            },
            json!({ "skill": { "id": "skill-1", "name": "review", "description": "Reviews implementation changes" } }),
        );
        assert_serialized_json(
            &GetSkillRequest {
                skill_id: "skill-1".to_string(),
            },
            json!({ "skillId": "skill-1" }),
        );
        assert_serialized_json(
            &GetSkillResponse {
                skill: skill.clone(),
            },
            json!({ "skill": { "id": "skill-1", "name": "review", "description": "Reviews implementation changes" } }),
        );
        assert_serialized_json(&ListSkillsRequest {}, json!({}));
        assert_serialized_json(
            &ListSkillsResponse {
                skills: vec![skill.clone()],
            },
            json!({ "skills": [{ "id": "skill-1", "name": "review", "description": "Reviews implementation changes" }] }),
        );
        assert_serialized_json(
            &UpdateSkillRequest {
                skill_id: "skill-1".to_string(),
                name: "code-review".to_string(),
                description: "Reviews code changes".to_string(),
            },
            json!({ "skillId": "skill-1", "name": "code-review", "description": "Reviews code changes" }),
        );
        assert_serialized_json(
            &UpdateSkillResponse {
                skill: Skill {
                    id: "skill-1".to_string(),
                    name: "code-review".to_string(),
                    description: "Reviews code changes".to_string(),
                },
            },
            json!({ "skill": { "id": "skill-1", "name": "code-review", "description": "Reviews code changes" } }),
        );
        assert_serialized_json(
            &DeleteSkillRequest {
                skill_id: "skill-1".to_string(),
            },
            json!({ "skillId": "skill-1" }),
        );
        assert_serialized_json(
            &DeleteSkillResponse {
                skill_id: "skill-1".to_string(),
            },
            json!({ "skillId": "skill-1" }),
        );
    }

    fn assert_serialized_json(value: &impl serde::Serialize, expected: serde_json::Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
