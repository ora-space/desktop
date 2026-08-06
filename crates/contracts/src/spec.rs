use crate::WorkspaceFileEventBatch;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Selects the project checkout or task workspace whose specification documents are managed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export_to = "spec.ts")]
pub enum SpecTarget {
    Project {
        #[serde(rename = "projectId")]
        #[ts(rename = "projectId")]
        project_id: String,
    },
    Task {
        #[serde(rename = "taskId")]
        #[ts(rename = "taskId")]
        task_id: String,
    },
}

/// Identifies the workflow that owns a specification source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export_to = "spec.ts")]
pub enum SpecWorkflow {
    OpenSpec,
    Superpowers,
    Custom { name: String },
}

/// Records whether a source came from Ora defaults, discovery, or an explicit user choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "spec.ts")]
pub enum SpecSourceOrigin {
    Default,
    Discovered,
    Manual,
}

/// Records whether a source participates in the effective catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "spec.ts")]
pub enum SpecSourceVisibility {
    Enabled,
    Disabled,
}

/// Records whether a configured source exists in the selected checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "spec.ts")]
pub enum SpecSourceAvailability {
    Available,
    Missing,
}

/// Describes one effective or disabled specification source in the selected context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct SpecSource {
    pub relative_path: String,
    pub workflow: SpecWorkflow,
    pub origin: SpecSourceOrigin,
    pub visibility: SpecSourceVisibility,
    pub availability: SpecSourceAvailability,
}

/// Describes one Markdown document assigned to an enabled source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct SpecDocument {
    pub relative_path: String,
    pub source_relative_path: String,
    pub workflow: SpecWorkflow,
    pub byte_size: u32,
}

/// Requests the bounded catalog for one project or task context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct GetSpecCatalogRequest {
    pub target: SpecTarget,
}

/// Returns source state plus the bounded Markdown document index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct SpecCatalogResponse {
    pub sources: Vec<SpecSource>,
    pub documents: Vec<SpecDocument>,
    pub truncated: bool,
}

/// Requests one catalog-authorized Markdown document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct ReadSpecRequest {
    pub target: SpecTarget,
    pub relative_path: String,
}

/// Returns the raw, read-only Markdown payload and its exact size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct ReadSpecResponse {
    pub relative_path: String,
    pub content: String,
    pub byte_size: u32,
}

/// Validates an absolute directory selected by the existing platform picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct ResolveSpecSourceRequest {
    pub target: SpecTarget,
    pub absolute_path: String,
}

/// Returns the normalized source path and workflow inferred from its segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct ResolveSpecSourceResponse {
    pub relative_path: String,
    pub workflow: SpecWorkflow,
}

/// Carries one project-level source override in an atomic replacement request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct ProjectSpecSourceOverride {
    pub relative_path: String,
    pub workflow: SpecWorkflow,
    pub visibility: SpecSourceVisibility,
}

/// Atomically replaces all source overrides owned by one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct UpdateProjectSpecSourcesRequest {
    pub project_id: String,
    pub sources: Vec<ProjectSpecSourceOverride>,
}

/// Returns the persisted replacement so adapters share one authoritative representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct UpdateProjectSpecSourcesResponse {
    pub sources: Vec<ProjectSpecSourceOverride>,
}

/// Starts specification-aware workspace file monitoring for one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "spec.ts")]
pub struct WatchSpecsRequest {
    pub target: SpecTarget,
}

/// Gives the stream event type a spec-owned name while retaining the shared wire format.
pub type WatchSpecsEvent = WorkspaceFileEventBatch;

/// Exports every TypeScript binding declared in this module.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    SpecTarget::export(config)?;
    SpecWorkflow::export(config)?;
    SpecSourceOrigin::export(config)?;
    SpecSourceVisibility::export(config)?;
    SpecSourceAvailability::export(config)?;
    SpecSource::export(config)?;
    SpecDocument::export(config)?;
    GetSpecCatalogRequest::export(config)?;
    SpecCatalogResponse::export(config)?;
    ReadSpecRequest::export(config)?;
    ReadSpecResponse::export(config)?;
    ResolveSpecSourceRequest::export(config)?;
    ResolveSpecSourceResponse::export(config)?;
    ProjectSpecSourceOverride::export(config)?;
    UpdateProjectSpecSourcesRequest::export(config)?;
    UpdateProjectSpecSourcesResponse::export(config)?;
    WatchSpecsRequest::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies target and custom workflow tagged unions retain the frontend wire shape.
    #[test]
    fn serializes_tagged_spec_contracts() {
        assert_eq!(
            serde_json::to_value(GetSpecCatalogRequest {
                target: SpecTarget::Task {
                    task_id: "task-1".to_string(),
                },
            })
            .unwrap(),
            json!({ "target": { "kind": "task", "taskId": "task-1" } })
        );
        assert_eq!(
            serde_json::to_value(SpecWorkflow::Custom {
                name: "Architecture".to_string(),
            })
            .unwrap(),
            json!({ "kind": "custom", "name": "Architecture" })
        );
    }
}
