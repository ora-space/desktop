use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Opaque host-issued authority attached to one plugin invocation.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export_to = "workbench.ts")]
pub struct PluginInvocationContext {
    pub id: String,
}

/// Host envelope carrying scoped invocation authority and page-supplied input.
#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq)]
#[ts(export_to = "workbench.ts")]
pub struct WorkbenchCallParams {
    pub context: PluginInvocationContext,
    #[ts(type = "import(\"./json.ts\").JsonValue")]
    pub input: Value,
}

/// Exports the workbench invocation DTO shared by the host and SDK.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PluginInvocationContext::export(config)?;
    WorkbenchCallParams::export(config)?;
    Ok(())
}
