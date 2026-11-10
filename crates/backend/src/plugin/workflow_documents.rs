//! Reads the workflow documents an imported or installed Workflow package contributes.
//!
//! This stays out of the plugin API itself so the plugin layer only has to hand over document
//! text. Deciding what a workflow is belongs to the workflow use case, which the caller sequences
//! once the package is committed.

use super::ContributedWorkflowDocument;
use crate::BackendError;
use ora_application::WorkflowDocument;
use ora_plugin_manager::{InstalledPlugin, PluginContribution, PluginManager};
use std::path::Path;

/// Reads every workflow document the freshly installed package contributes.
///
/// A package of any other kind contributes no documents, which reports as an empty list rather
/// than an error so the caller needs no knowledge of which kinds carry workflows.
pub(crate) fn read_workflow_documents(
    home_directory: &Path,
    plugin_id: &str,
) -> Result<Vec<WorkflowDocument>, BackendError> {
    let manager = PluginManager::discover(home_directory);
    let Some(plugin) = manager
        .installed_plugins()
        .iter()
        .find(|plugin| plugin.id.canonical() == plugin_id)
    else {
        return Ok(Vec::new());
    };
    read_package_documents(plugin)
}

/// Reads the documents every plugin in `plugin_ids` contributes, each attributed to its member.
///
/// One discovery pass serves the whole batch, because a pack install asks about several members at
/// once and re-scanning the plugin tree per member would multiply that cost for no benefit. A
/// member that is not installed, or whose package carries no documents, simply contributes nothing
/// rather than failing the operation the caller already reported on.
pub(crate) fn read_contributed_workflow_documents(
    home_directory: &Path,
    plugin_ids: &[String],
) -> Result<Vec<ContributedWorkflowDocument>, BackendError> {
    let manager = PluginManager::discover(home_directory);
    let installed = manager.installed_plugins();
    let mut contributed = Vec::new();
    for plugin_id in plugin_ids {
        let Some(plugin) = installed
            .iter()
            .find(|plugin| plugin.id.canonical() == *plugin_id)
        else {
            continue;
        };
        for document in read_package_documents(plugin)? {
            contributed.push(ContributedWorkflowDocument {
                namespace: plugin.id.namespace().to_string(),
                identifier: plugin.id.name().to_string(),
                document,
            });
        }
    }
    Ok(contributed)
}

/// Reads one discovered package's workflow documents in the order discovery reports them.
fn read_package_documents(plugin: &InstalledPlugin) -> Result<Vec<WorkflowDocument>, BackendError> {
    let PluginContribution::Workflow(descriptor) = &plugin.contributes else {
        return Ok(Vec::new());
    };

    let mut documents = Vec::with_capacity(descriptor.files.len());
    for file in &descriptor.files {
        let path = plugin.package_root.join(file.to_path_buf());
        // A read failure here is not a per-document content problem: discovery already proved
        // this is a regular file inside the package it just extracted, so failing to read it
        // means the data directory itself is unusable. The IO error alone does not name the file,
        // and the file is the whole point of the report.
        let contents = std::fs::read_to_string(&path).map_err(|error| {
            BackendError::internal(
                "failed to read an installed workflow document",
                std::io::Error::new(error.kind(), format!("{}: {error}", path.display())),
            )
        })?;
        documents.push(WorkflowDocument {
            source_file: file.as_str().to_string(),
            contents,
        });
    }
    Ok(documents)
}
