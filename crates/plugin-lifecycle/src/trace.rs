//! Serves context-bound `ora/trace/*` requests without exposing host filesystem paths.

use crate::connection::PluginGenerationKey;
use crate::context::{AuthorizedTrace, PluginInvocationContexts, StoredContext};
use base64::Engine;
use ora_domain::PluginId;
use ora_plugin_runtime::{HostRequestError, PluginTraceProvider, PluginTraceRoot};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use uuid::Uuid;

pub const TRACE_LIST_METHOD: &str = "ora/trace/list";
pub const TRACE_STAT_METHOD: &str = "ora/trace/stat";
pub const TRACE_READ_METHOD: &str = "ora/trace/read";
pub const MAX_TRACE_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISCOVERY_ENTRIES: usize = 100_000;
const MAX_DISCOVERY_DEPTH: usize = 8;

/// Per-process handler; caller identity is captured at launch and never read from params.
#[derive(Debug, Clone)]
pub struct PluginTraceHost {
    contexts: PluginInvocationContexts,
    caller_plugin_id: PluginId,
    caller_generation: PluginGenerationKey,
    user_home: PathBuf,
}

impl PluginTraceHost {
    pub fn new(
        contexts: PluginInvocationContexts,
        caller_plugin_id: PluginId,
        caller_generation: PluginGenerationKey,
        user_home: PathBuf,
    ) -> Self {
        Self {
            contexts,
            caller_plugin_id,
            caller_generation,
            user_home,
        }
    }

    pub async fn handle(&self, method: &str, params: Value) -> Result<Value, HostRequestError> {
        let context_id = required_string(&params, "context_id")?;
        self.contexts
            .with_context(
                context_id,
                &self.caller_plugin_id,
                self.caller_generation,
                |context| match method {
                    TRACE_LIST_METHOD => self.list(context),
                    TRACE_STAT_METHOD => self.stat(context, &params),
                    TRACE_READ_METHOD => self.read(context, &params),
                    _ => Err(HostRequestError::method_not_found(method)),
                },
            )
            .ok_or_else(|| trace_error("context_unavailable", "trace context is unavailable"))?
    }

    fn list(&self, context: &mut StoredContext) -> Result<Value, HostRequestError> {
        let discovered = self.discover(context)?;
        context.traces = discovered
            .iter()
            .cloned()
            .map(|trace| (trace.trace_id.clone(), trace))
            .collect();
        let traces = discovered
            .iter()
            .map(trace_metadata)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "traces": traces }))
    }

    fn stat(&self, context: &mut StoredContext, params: &Value) -> Result<Value, HostRequestError> {
        let trace_id = required_string(params, "trace_id")?;
        let trace = context
            .traces
            .get(trace_id)
            .ok_or_else(|| trace_error("trace_not_found", "trace is not in this context"))?;
        trace_metadata(trace)
    }

    fn read(&self, context: &mut StoredContext, params: &Value) -> Result<Value, HostRequestError> {
        let trace_id = required_string(params, "trace_id")?;
        let offset = required_u64(params, "offset")?;
        let max_bytes = required_u64(params, "max_bytes")? as usize;
        if max_bytes == 0 || max_bytes > MAX_TRACE_CHUNK_BYTES {
            return Err(invalid_params(format!(
                "max_bytes must be between 1 and {MAX_TRACE_CHUNK_BYTES}"
            )));
        }
        let trace = context
            .traces
            .get(trace_id)
            .ok_or_else(|| trace_error("trace_not_found", "trace is not in this context"))?;
        verify_containment(trace)?;
        let metadata = trace.path.metadata().map_err(map_io)?;
        let current_cursor = cursor(&metadata);
        if let Some(expected) = optional_string(params, "cursor")?
            && expected != current_cursor
        {
            return Err(trace_error(
                "stale_cursor",
                "trace was replaced or truncated; list it again",
            ));
        }
        if offset > metadata.len() {
            return Err(trace_error(
                "stale_cursor",
                "trace is shorter than the requested offset",
            ));
        }
        let mut file = File::open(&trace.path).map_err(map_io)?;
        file.seek(SeekFrom::Start(offset)).map_err(map_io)?;
        let available = metadata.len().saturating_sub(offset);
        let wanted = available.min(max_bytes as u64) as usize;
        let mut bytes = vec![0; wanted];
        file.read_exact(&mut bytes).map_err(map_io)?;
        let next_offset = offset + bytes.len() as u64;
        Ok(json!({
            "bytes_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
            "offset": offset,
            "next_offset": next_offset,
            "eof": next_offset >= metadata.len(),
            "cursor": current_cursor,
        }))
    }

    fn discover(
        &self,
        context: &mut StoredContext,
    ) -> Result<Vec<AuthorizedTrace>, HostRequestError> {
        let grant = context
            .trace
            .clone()
            .ok_or_else(|| trace_error("trace_unavailable", "context has no trace capability"))?;
        let mut traces = Vec::new();
        for session in &grant.sessions {
            for provider in &session.providers {
                let root = match provider.locator.root {
                    PluginTraceRoot::Home => &self.user_home,
                    PluginTraceRoot::Workspace => &session.workspace_root,
                };
                let root = root.canonicalize().map_err(map_io)?;
                let directory = root.join(&provider.locator.directory);
                let directory = match directory.canonicalize() {
                    Ok(directory) if directory.starts_with(&root) => directory,
                    Ok(_) => {
                        return Err(trace_error(
                            "invalid_locator",
                            "trace directory escapes its declared root",
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(map_io(error)),
                };
                let file_name = provider
                    .locator
                    .file_name_template
                    .replace("{provider_session_id}", &session.provider_session_id);
                if !safe_file_name(&file_name) {
                    return Err(trace_error(
                        "invalid_locator",
                        "provider session produced an unsafe trace file name",
                    ));
                }
                if provider.locator.recursive {
                    discover_recursive(
                        &directory,
                        &file_name,
                        provider,
                        &session.label,
                        session.ora_session_id == grant.current_ora_session_id,
                        &mut context.trace_ids,
                        &mut traces,
                    )?;
                } else {
                    add_trace_if_file(
                        &directory.join(file_name),
                        &directory,
                        provider,
                        &session.label,
                        session.ora_session_id == grant.current_ora_session_id,
                        &mut context.trace_ids,
                        &mut traces,
                    )?;
                }
            }
        }
        Ok(traces)
    }
}

fn discover_recursive(
    root: &Path,
    file_name: &str,
    provider: &PluginTraceProvider,
    label: &str,
    is_current: bool,
    trace_ids: &mut std::collections::HashMap<String, String>,
    traces: &mut Vec<AuthorizedTrace>,
) -> Result<(), HostRequestError> {
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_DISCOVERY_DEPTH || visited >= MAX_DISCOVERY_ENTRIES {
            continue;
        }
        for entry in std::fs::read_dir(&directory).map_err(map_io)? {
            let entry = entry.map_err(map_io)?;
            visited += 1;
            if visited > MAX_DISCOVERY_ENTRIES {
                break;
            }
            let file_type = entry.file_type().map_err(map_io)?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push((entry.path(), depth + 1));
            } else if file_type.is_file() && entry.file_name() == file_name {
                add_trace_if_file(
                    &entry.path(),
                    root,
                    provider,
                    label,
                    is_current,
                    trace_ids,
                    traces,
                )?;
            }
        }
    }
    Ok(())
}

fn add_trace_if_file(
    path: &Path,
    containment_root: &Path,
    provider: &PluginTraceProvider,
    label: &str,
    is_current: bool,
    trace_ids: &mut std::collections::HashMap<String, String>,
    traces: &mut Vec<AuthorizedTrace>,
) -> Result<(), HostRequestError> {
    if !path.is_file() {
        return Ok(());
    }
    let canonical = path.canonicalize().map_err(map_io)?;
    if !canonical.starts_with(containment_root) {
        return Err(trace_error(
            "invalid_locator",
            "trace file escapes its declared directory",
        ));
    }
    traces.push(AuthorizedTrace {
        trace_id: trace_ids
            .entry(canonical.to_string_lossy().to_string())
            .or_insert_with(|| Uuid::new_v4().to_string())
            .clone(),
        provider_id: provider.provider_id.clone(),
        format: provider.format.clone(),
        path: canonical,
        containment_root: containment_root.to_path_buf(),
        label: label.to_string(),
        is_current,
    });
    Ok(())
}

fn verify_containment(trace: &AuthorizedTrace) -> Result<(), HostRequestError> {
    let canonical = trace.path.canonicalize().map_err(map_io)?;
    if canonical != trace.path || !canonical.starts_with(&trace.containment_root) {
        return Err(trace_error(
            "trace_unavailable",
            "trace path changed after authorization",
        ));
    }
    Ok(())
}

fn trace_metadata(trace: &AuthorizedTrace) -> Result<Value, HostRequestError> {
    verify_containment(trace)?;
    let metadata = trace.path.metadata().map_err(map_io)?;
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    Ok(json!({
        "trace_id": trace.trace_id,
        "provider_id": trace.provider_id,
        "format": trace.format,
        "size_bytes": metadata.len(),
        "modified_at_ms": modified_at_ms,
        "cursor": cursor(&metadata),
        "label": trace.label,
        "is_current": trace.is_current,
    }))
}

fn cursor(metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{modified}:{}", metadata.len())
}

fn safe_file_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

fn required_string<'a>(params: &'a Value, field: &str) -> Result<&'a str, HostRequestError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_params(format!("{field} must be a non-empty string")))
}

fn optional_string<'a>(
    params: &'a Value,
    field: &str,
) -> Result<Option<&'a str>, HostRequestError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(invalid_params(format!("{field} must be a string or null"))),
    }
}

fn required_u64(params: &Value, field: &str) -> Result<u64, HostRequestError> {
    params
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_params(format!("{field} must be a non-negative integer")))
}

fn invalid_params(message: impl Into<String>) -> HostRequestError {
    HostRequestError::new(-32602, message).with_data(json!({ "kind": "invalid_params" }))
}

fn trace_error(kind: &str, message: impl Into<String>) -> HostRequestError {
    HostRequestError::new(-32020, message).with_data(json!({ "kind": kind }))
}

fn map_io(error: std::io::Error) -> HostRequestError {
    let kind = if error.kind() == std::io::ErrorKind::NotFound {
        "trace_not_found"
    } else {
        "io"
    };
    trace_error(kind, "trace file is unavailable")
}
