use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Reports whether a catalog skill still has a loadable on-disk package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "skill.ts")]
pub enum SkillAvailability {
    Available,
    Unavailable,
}

/// Describes a public skill payload without persistence audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct Skill {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub description: String,
    pub availability: SkillAvailability,
}

/// Describes one skill together with the Markdown body from its SKILL.md.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct SkillDetails {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub description: String,
    pub content: String,
    pub availability: SkillAvailability,
}

/// Carries the public fields required to create a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill.ts")]
pub struct CreateSkillRequest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    #[ts(optional)]
    pub content: Option<String>,
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
    pub skill: SkillDetails,
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
    #[serde(default)]
    #[ts(optional)]
    pub content: Option<String>,
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
    SkillAvailability::export(config)?;
    Skill::export(config)?;
    SkillDetails::export(config)?;
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
        SkillAvailability, UpdateSkillRequest, UpdateSkillResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies public skill payloads exclude persistence-owned audit fields.
    #[test]
    fn serializes_skill_contract_without_audit_fields() {
        let skill = Skill {
            id: "skill-1".to_string(),
            namespace: "local".to_string(),
            name: "review".to_string(),
            description: "Reviews implementation changes".to_string(),
            availability: SkillAvailability::Available,
        };

        assert_eq!(
            serde_json::to_value(skill).unwrap(),
            json!({
                "id": "skill-1",
                "namespace": "local",
                "name": "review",
                "description": "Reviews implementation changes",
                "availability": "available",
            })
        );
    }

    /// Verifies skill CRUD requests preserve the resource identifier separately from editable fields.
    #[test]
    fn serializes_skill_crud_contracts() {
        let skill = Skill {
            id: "skill-1".to_string(),
            namespace: "local".to_string(),
            name: "review".to_string(),
            description: "Reviews implementation changes".to_string(),
            availability: SkillAvailability::Available,
        };

        assert_serialized_json(
            &CreateSkillRequest {
                name: skill.name.clone(),
                description: skill.description.clone(),
                content: Some("# Instructions".to_string()),
            },
            json!({ "name": "review", "description": "Reviews implementation changes", "content": "# Instructions" }),
        );
        assert_serialized_json(
            &CreateSkillResponse {
                skill: skill.clone(),
            },
            json!({ "skill": { "id": "skill-1", "namespace": "local", "name": "review", "description": "Reviews implementation changes", "availability": "available" } }),
        );
        assert_serialized_json(
            &GetSkillRequest {
                skill_id: "skill-1".to_string(),
            },
            json!({ "skillId": "skill-1" }),
        );
        assert_serialized_json(
            &GetSkillResponse {
                skill: super::SkillDetails {
                    id: skill.id.clone(),
                    namespace: skill.namespace.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    content: "# Instructions".to_string(),
                    availability: SkillAvailability::Available,
                },
            },
            json!({ "skill": { "id": "skill-1", "namespace": "local", "name": "review", "description": "Reviews implementation changes", "content": "# Instructions", "availability": "available" } }),
        );
        assert_serialized_json(&ListSkillsRequest {}, json!({}));
        assert_serialized_json(
            &ListSkillsResponse {
                skills: vec![skill.clone()],
            },
            json!({ "skills": [{ "id": "skill-1", "namespace": "local", "name": "review", "description": "Reviews implementation changes", "availability": "available" }] }),
        );
        assert_serialized_json(
            &UpdateSkillRequest {
                skill_id: "skill-1".to_string(),
                name: "code-review".to_string(),
                description: "Reviews code changes".to_string(),
                content: Some("Updated instructions".to_string()),
            },
            json!({ "skillId": "skill-1", "name": "code-review", "description": "Reviews code changes", "content": "Updated instructions" }),
        );
        let legacy_create: CreateSkillRequest = serde_json::from_value(json!({
            "name": "legacy",
            "description": "Legacy skill"
        }))
        .unwrap();
        assert_eq!(legacy_create.content, None);
        let legacy_update: UpdateSkillRequest = serde_json::from_value(json!({
            "skillId": "skill-1",
            "name": "legacy",
            "description": "Legacy skill"
        }))
        .unwrap();
        assert_eq!(legacy_update.content, None);
        assert_serialized_json(
            &UpdateSkillResponse {
                skill: Skill {
                    id: "skill-1".to_string(),
                    namespace: "local".to_string(),
                    name: "code-review".to_string(),
                    description: "Reviews code changes".to_string(),
                    availability: SkillAvailability::Available,
                },
            },
            json!({ "skill": { "id": "skill-1", "namespace": "local", "name": "code-review", "description": "Reviews code changes", "availability": "available" } }),
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
