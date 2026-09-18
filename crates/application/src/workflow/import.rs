use std::sync::Arc;

use ora_contracts::{CreateWorkflowRequest, ImportedWorkflowOutcome, PublishWorkflowRequest};
use serde::Deserialize;

use crate::Clock;
use crate::workflow::handlers::{CreateWorkflowHandler, PublishWorkflowHandler};
use crate::workflow::ports::{WorkflowIdGenerator, WorkflowRepository};
use crate::workflow::version::is_publishable_version;
use crate::workflow_run::WorkflowGraph;

/// Export naming suffixes the editor writes and a packager may leave on a document file name.
const DOCUMENT_SUFFIXES: [&str; 2] = [".reactflow.json", ".json"];

/// Holds one workflow document read from a package, paired with the path it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDocument {
    /// Package-relative path the document was read from, reported back with its outcome.
    pub source_file: String,
    /// The document's text, taken from the package without rewriting it.
    pub contents: String,
}

/// Imports workflow documents into the library, one workflow and one published snapshot each.
///
/// The use case sequences the existing create and publish handlers instead of reimplementing
/// them, so an imported document passes exactly the name-conflict, graph, and version rules an
/// editor-saved workflow passes. Documents are imported independently: a failure becomes that
/// document's outcome and never aborts the batch, because a package that carries one broken
/// workflow should still deliver the rest.
pub struct ImportWorkflowsHandler<Repository, IdGenerator, ClockSource> {
    create: CreateWorkflowHandler<Repository, IdGenerator, ClockSource>,
    publish: PublishWorkflowHandler<Repository, IdGenerator, ClockSource>,
}

impl<Repository, IdGenerator, ClockSource>
    ImportWorkflowsHandler<Repository, IdGenerator, ClockSource>
where
    Repository: WorkflowRepository + Clone,
    IdGenerator: WorkflowIdGenerator + Clone,
    ClockSource: Clock + Clone,
{
    /// Builds the import use case from the same collaborators the editor-facing handlers use.
    pub fn new(repository: Arc<Repository>, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            // Create owns its repository by value while publish shares it, mirroring how the
            // backend composes the same two handlers for the editor.
            create: CreateWorkflowHandler::new(
                (*repository).clone(),
                id_generator.clone(),
                clock.clone(),
            ),
            publish: PublishWorkflowHandler::new(repository, id_generator, clock),
        }
    }
}

impl<Repository, IdGenerator, ClockSource>
    ImportWorkflowsHandler<Repository, IdGenerator, ClockSource>
where
    Repository: WorkflowRepository + Clone + Send + Sync + 'static,
    IdGenerator: WorkflowIdGenerator,
    ClockSource: Clock,
{
    /// Imports every document, returning one outcome per document in the order given.
    pub fn handle(&self, documents: Vec<WorkflowDocument>) -> Vec<ImportedWorkflowOutcome> {
        documents
            .into_iter()
            .map(|document| self.import_one(document))
            .collect()
    }

    /// Imports one document, turning every failure into that document's own outcome.
    fn import_one(&self, document: WorkflowDocument) -> ImportedWorkflowOutcome {
        match self.try_import(&document) {
            Ok(outcome) => outcome,
            Err(reason) => ImportedWorkflowOutcome::Failed {
                source_file: document.source_file,
                reason,
            },
        }
    }

    /// Creates and publishes the workflow one document describes.
    fn try_import(&self, document: &WorkflowDocument) -> Result<ImportedWorkflowOutcome, String> {
        let declared: DeclaredDocument = serde_json::from_str(&document.contents)
            .map_err(|error| format!("document is not valid JSON: {error}"))?;
        let name = declared.name.as_deref().unwrap_or_default().trim();
        if name.is_empty() {
            return Err("document declares no workflow name".to_string());
        }
        // The run engine owns graph validation, so a document that could never execute is refused
        // here rather than stored and discovered when a run starts.
        WorkflowGraph::parse(&document.contents)
            .map_err(|error| format!("document is not an executable workflow graph: {error}"))?;

        let created = self
            .create
            .handle(CreateWorkflowRequest {
                name: name.to_string(),
                // The stored graph is the document verbatim. The engine ignores the editor
                // metadata written beside the graph, so no field needs projecting out first, and
                // keeping the document intact means an exported file round-trips unchanged.
                graph: Some(document.contents.clone()),
            })
            .map_err(|error| error.to_string())?;

        let version =
            derive_publish_version(declared.version.as_deref(), &document.source_file, name);
        let published = self
            .publish
            .handle(PublishWorkflowRequest {
                workflow_id: created.workflow.id.clone(),
                version,
            })
            .map_err(|error| error.to_string())?;

        Ok(ImportedWorkflowOutcome::Imported {
            source_file: document.source_file.clone(),
            workflow_id: created.workflow.id,
            name: name.to_string(),
            version: published.snapshot.version,
        })
    }
}

/// The fields import reads out of a document before storing the rest as the graph.
#[derive(Debug, Deserialize)]
struct DeclaredDocument {
    /// Workflow name; the created `Workflow` record owns this value.
    name: Option<String>,
    /// Explicit publish version, preferred over the file name.
    version: Option<String>,
}

/// Picks the published version for one imported document.
///
/// Precedence is the document's own `version` field, then the name of the file it was packaged
/// under, then the workflow title. `None` means the caller should let the publish handler mint an
/// automatic version. A candidate that is not addressable is refused rather than sanitized,
/// because silently rewriting a version would publish under a name its author never wrote.
pub(crate) fn derive_publish_version(
    explicit: Option<&str>,
    source_file: &str,
    workflow_name: &str,
) -> Option<String> {
    if let Some(version) = explicit
        && is_publishable_version(version)
    {
        return Some(version.to_owned());
    }
    let stem = document_stem(source_file);
    let candidate = if stem.is_empty() {
        workflow_name.trim()
    } else {
        stem
    };
    is_publishable_version(candidate).then(|| candidate.to_owned())
}

/// Returns the file name a document was packaged under with its export suffix removed.
///
/// The editor writes `<workflow>.reactflow.json`; a package may also carry a plain `.json`. The
/// suffix comparison ignores ASCII case because the package layout check accepts either spelling,
/// and the longest suffix is tried first so `.reactflow.json` never loses to `.json`.
fn document_stem(source_file: &str) -> &str {
    let file_name = source_file.rsplit('/').next().unwrap_or(source_file).trim();
    let lowered = file_name.to_ascii_lowercase();
    match DOCUMENT_SUFFIXES
        .iter()
        .find(|suffix| lowered.ends_with(*suffix))
    {
        Some(suffix) => file_name[..file_name.len() - suffix.len()].trim(),
        None => file_name,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ora_contracts::ImportedWorkflowOutcome;
    use ora_domain::{
        CreatedWorkflow, Namespace, Workflow, WorkflowDetail, WorkflowId, WorkflowSnapshot,
        WorkflowSnapshotId, WorkflowSummary, WorkflowVersion,
    };
    use pretty_assertions::assert_eq;

    use super::{ImportWorkflowsHandler, WorkflowDocument, derive_publish_version};
    use crate::workflow::ports::{
        ActivateVersionResult, DeleteSnapshotResult, DeleteWorkflowResult, PublishSnapshotResult,
        RollbackDraftResult, UpdateDraftResult, UpdateWorkflowResult, WorkflowIdGenerator,
        WorkflowRepository,
    };
    use crate::{Clock, RepositoryError};

    /// One Start-only graph the run engine accepts.
    const VALID_GRAPH: &str = r#"{"nodes":[{"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","title":"开始"}}],"edges":[]}"#;

    /// A graph whose edge names a node that does not exist, which the engine refuses.
    const DANGLING_EDGE_GRAPH: &str = r#"{"name":"坏图","nodes":[{"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","title":"开始"}}],"edges":[{"source":"start","target":"ghost"}]}"#;

    /// Builds one document payload with the given name and optional explicit version.
    fn document_json(name: Option<&str>, version: Option<&str>) -> String {
        let graph: serde_json::Value = serde_json::from_str(VALID_GRAPH).unwrap();
        let mut value = serde_json::Map::new();
        if let Some(name) = name {
            value.insert("name".to_string(), serde_json::Value::from(name));
        }
        if let Some(version) = version {
            value.insert("version".to_string(), serde_json::Value::from(version));
        }
        for (key, entry) in graph.as_object().unwrap() {
            value.insert(key.clone(), entry.clone());
        }
        serde_json::Value::Object(value).to_string()
    }

    /// Wraps one payload as a package-relative document.
    fn document(source_file: &str, contents: String) -> WorkflowDocument {
        WorkflowDocument {
            source_file: source_file.to_string(),
            contents,
        }
    }

    /// Records what import created and published so outcomes can be asserted against it.
    ///
    /// The inner state sits behind `Arc` because the handler clones the repository for its create
    /// half while its publish half shares the original; a deep clone would let the two halves
    /// disagree about what has already been written.
    #[derive(Debug, Default, Clone)]
    struct ImportRepository {
        created: Arc<Mutex<Vec<(String, Workflow)>>>,
        published: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl WorkflowRepository for ImportRepository {
        fn create_workflow(
            &self,
            workflow: Workflow,
            draft: WorkflowSnapshot,
        ) -> Result<CreatedWorkflow, RepositoryError> {
            self.created
                .lock()
                .unwrap()
                .push((workflow.name.to_ascii_lowercase(), workflow.clone()));
            Ok(CreatedWorkflow { workflow, draft })
        }

        fn find_workflow_by_name(
            &self,
            _namespace: &Namespace,
            name: &str,
        ) -> Result<Option<Workflow>, RepositoryError> {
            Ok(self
                .created
                .lock()
                .unwrap()
                .iter()
                .find(|(created, _)| created == &name.to_ascii_lowercase())
                .map(|(_, workflow)| workflow.clone()))
        }

        fn publish_snapshot(
            &self,
            workflow_id: &WorkflowId,
            snapshot_id: WorkflowSnapshotId,
            version: String,
            created_at: i64,
        ) -> Result<PublishSnapshotResult, RepositoryError> {
            self.published
                .lock()
                .unwrap()
                .push((workflow_id.to_string(), version.clone()));
            Ok(PublishSnapshotResult::Published(WorkflowSnapshot {
                id: snapshot_id,
                workflow_id: workflow_id.clone(),
                version,
                graph: VALID_GRAPH.to_string(),
                created_at,
                updated_at: None,
                is_deleted: false,
            }))
        }

        fn find_workflow(
            &self,
            _workflow_id: &WorkflowId,
        ) -> Result<Option<Workflow>, RepositoryError> {
            unreachable!("import never loads a workflow by id")
        }

        fn get_workflow_detail(
            &self,
            _workflow_id: &WorkflowId,
        ) -> Result<Option<WorkflowDetail>, RepositoryError> {
            unreachable!("import never loads workflow details")
        }

        fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, RepositoryError> {
            unreachable!("import never lists workflows")
        }

        fn update_workflow(
            &self,
            _workflow_id: &WorkflowId,
            _name: String,
            _updated_at: i64,
        ) -> Result<UpdateWorkflowResult, RepositoryError> {
            unreachable!("import never renames a workflow")
        }

        fn soft_delete_workflow(
            &self,
            _workflow_id: &WorkflowId,
            _deleted_at: i64,
        ) -> Result<DeleteWorkflowResult, RepositoryError> {
            unreachable!("import never deletes a workflow")
        }

        fn find_snapshot_by_version(
            &self,
            _workflow_id: &WorkflowId,
            _version: &str,
        ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
            unreachable!("import never looks up a snapshot by version")
        }

        fn find_snapshot_by_id(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
        ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
            unreachable!("import never looks up a snapshot by id")
        }

        fn find_snapshot_any_workflow(
            &self,
            _snapshot_id: &WorkflowSnapshotId,
        ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
            unreachable!("import never looks up a snapshot across workflows")
        }

        fn list_versions(
            &self,
            _workflow_id: &WorkflowId,
        ) -> Result<Vec<WorkflowVersion>, RepositoryError> {
            unreachable!("import never lists versions")
        }

        fn update_draft(
            &self,
            _workflow_id: &WorkflowId,
            _graph: String,
            _updated_at: i64,
        ) -> Result<UpdateDraftResult, RepositoryError> {
            unreachable!("import never edits a draft")
        }

        fn rollback_draft(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
            _updated_at: i64,
        ) -> Result<RollbackDraftResult, RepositoryError> {
            unreachable!("import never rolls a draft back")
        }

        fn activate_version(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
            _updated_at: i64,
        ) -> Result<ActivateVersionResult, RepositoryError> {
            unreachable!("import never activates a version")
        }

        fn soft_delete_snapshot(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
            _deleted_at: i64,
        ) -> Result<DeleteSnapshotResult, RepositoryError> {
            unreachable!("import never deletes a snapshot")
        }
    }

    /// Hands out distinct identifiers so a multi-document import never reuses one.
    #[derive(Debug, Default, Clone)]
    struct CountingIdGenerator {
        next: Arc<Mutex<u64>>,
    }

    impl WorkflowIdGenerator for CountingIdGenerator {
        fn generate_workflow_id(&self) -> WorkflowId {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            WorkflowId::new(format!("workflow-{next}"))
        }

        fn generate_snapshot_id(&self) -> WorkflowSnapshotId {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            WorkflowSnapshotId::new(format!("snapshot-{next}"))
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct FixedClock(i64);

    impl Clock for FixedClock {
        fn now_timestamp_millis(&self) -> i64 {
            self.0
        }
    }

    /// Builds a handler over a fresh recording repository.
    fn handler() -> ImportWorkflowsHandler<ImportRepository, CountingIdGenerator, FixedClock> {
        ImportWorkflowsHandler::new(
            Arc::new(ImportRepository::default()),
            CountingIdGenerator::default(),
            FixedClock(42),
        )
    }

    /// Every document in a well-formed package becomes a workflow with a published snapshot.
    ///
    /// The identifier sequence reflects that one document draws three identifiers: create takes a
    /// workflow id and a draft snapshot id, then publish takes the published snapshot id.
    #[test]
    fn imports_each_document_and_publishes_a_snapshot() {
        let handler = handler();

        let outcomes = handler.handle(vec![
            document(
                "assets/workflows/1.0.0.json",
                document_json(Some("发布流程"), None),
            ),
            document(
                "assets/workflows/2.0.0.json",
                document_json(Some("回归流程"), None),
            ),
        ]);

        assert_eq!(
            outcomes,
            vec![
                ImportedWorkflowOutcome::Imported {
                    source_file: "assets/workflows/1.0.0.json".to_string(),
                    workflow_id: "workflow-1".to_string(),
                    name: "发布流程".to_string(),
                    version: "1.0.0".to_string(),
                },
                ImportedWorkflowOutcome::Imported {
                    source_file: "assets/workflows/2.0.0.json".to_string(),
                    workflow_id: "workflow-4".to_string(),
                    name: "回归流程".to_string(),
                    version: "2.0.0".to_string(),
                },
            ]
        );
    }

    /// One broken document is reported on its own and leaves the documents around it imported.
    #[test]
    fn reports_one_bad_document_without_stopping_the_batch() {
        let handler = handler();

        let outcomes = handler.handle(vec![
            document(
                "assets/workflows/1.0.0.json",
                document_json(Some("好流程"), None),
            ),
            document("assets/workflows/2.0.0.json", "{ not json".to_string()),
            document(
                "assets/workflows/3.0.0.json",
                document_json(Some("更好的流程"), None),
            ),
        ]);

        assert_eq!(outcomes.len(), 3);
        assert!(matches!(
            outcomes[0],
            ImportedWorkflowOutcome::Imported { .. }
        ));
        let ImportedWorkflowOutcome::Failed {
            source_file,
            reason,
        } = &outcomes[1]
        else {
            panic!(
                "expected the malformed document to fail, got {:?}",
                outcomes[1]
            )
        };
        assert_eq!(source_file, "assets/workflows/2.0.0.json");
        assert!(reason.contains("not valid JSON"), "{reason}");
        assert!(matches!(
            outcomes[2],
            ImportedWorkflowOutcome::Imported { .. }
        ));
    }

    /// A document without a usable name is refused before anything is created.
    #[test]
    fn refuses_documents_without_a_name() {
        let handler = handler();

        for payload in [document_json(None, None), document_json(Some("   "), None)] {
            let outcomes = handler.handle(vec![document("assets/workflows/1.0.0.json", payload)]);
            let ImportedWorkflowOutcome::Failed { reason, .. } = &outcomes[0] else {
                panic!(
                    "expected a nameless document to fail, got {:?}",
                    outcomes[0]
                )
            };
            assert_eq!(reason, "document declares no workflow name");
        }
    }

    /// A graph the run engine could not execute is refused rather than stored.
    #[test]
    fn refuses_documents_whose_graph_cannot_execute() {
        let handler = handler();

        let outcomes = handler.handle(vec![document(
            "assets/workflows/1.0.0.json",
            DANGLING_EDGE_GRAPH.to_string(),
        )]);

        let ImportedWorkflowOutcome::Failed { reason, .. } = &outcomes[0] else {
            panic!("expected a dangling edge to fail, got {:?}", outcomes[0])
        };
        assert!(
            reason.contains("not an executable workflow graph"),
            "{reason}"
        );
    }

    /// Two documents claiming one workflow name: the second fails and the first still lands.
    #[test]
    fn refuses_a_duplicate_workflow_name() {
        let handler = handler();

        let outcomes = handler.handle(vec![
            document(
                "assets/workflows/1.0.0.json",
                document_json(Some("同名"), None),
            ),
            document(
                "assets/workflows/2.0.0.json",
                document_json(Some("同名"), None),
            ),
        ]);

        assert!(matches!(
            outcomes[0],
            ImportedWorkflowOutcome::Imported { .. }
        ));
        let ImportedWorkflowOutcome::Failed { reason, .. } = &outcomes[1] else {
            panic!("expected the duplicate name to fail, got {:?}", outcomes[1])
        };
        assert!(reason.contains("同名"), "{reason}");
    }

    /// Version precedence is the document field, then the file name, then the workflow title.
    #[test]
    fn derives_publish_versions_in_precedence_order() {
        let cases = [
            (
                Some("9.9.9"),
                "assets/workflows/1.0.0.json",
                "标题",
                Some("9.9.9"),
            ),
            (None, "assets/workflows/1.0.0.json", "标题", Some("1.0.0")),
            (
                None,
                "assets/workflows/release.reactflow.json",
                "标题",
                Some("release"),
            ),
            (None, "assets/workflows/.json", "标题", Some("标题")),
            (None, "assets/workflows/draft.json", "标题", None),
            (None, "assets/workflows/..json", "标题", None),
            (None, "assets/workflows/.json", "  ", None),
            (
                Some("draft"),
                "assets/workflows/1.0.0.json",
                "标题",
                Some("1.0.0"),
            ),
            (
                Some("a/b"),
                "assets/workflows/1.0.0.json",
                "标题",
                Some("1.0.0"),
            ),
        ];

        for (explicit, source_file, name, expected) in cases {
            assert_eq!(
                derive_publish_version(explicit, source_file, name),
                expected.map(str::to_string),
                "explicit={explicit:?} file={source_file} name={name}",
            );
        }
    }
}
