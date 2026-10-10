//! OpenCode config uses environment references so serialization cannot persist the runtime token.

use super::{Grant, ModelAccessError, Protocol};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

/// Builds one provider and isolated CLI state while leaving model identifiers unmodified.
pub(super) fn environment(
    grant: &Grant,
    ca: &Path,
    node_home: &Path,
) -> Result<(BTreeMap<String, String>, tempfile::TempDir), ModelAccessError> {
    let runtime_root = node_home.join("model-runtime");
    ora_utils::path::create_directories_without_symlinks(&runtime_root)
        .map_err(|_| ModelAccessError("model_runtime_state_unavailable"))?;
    let home = tempfile::Builder::new()
        .prefix("opencode-")
        .tempdir_in(runtime_root)
        .map_err(|_| ModelAccessError("model_runtime_state_unavailable"))?;
    let model_id = &grant.model.id;
    let config = json!({
        "$schema": "https://opencode.ai/config.json",
        "model": format!("ora-model/{model_id}"),
        "small_model": format!("ora-model/{model_id}"),
        "enabled_providers": ["ora-model"],
        "provider": {"ora-model": {
            "npm": match grant.protocol {
                Protocol::OpenaiCompletions => "@ai-sdk/openai-compatible",
                Protocol::AnthropicMessages => "@ai-sdk/anthropic",
            },
            "name": "Ora model gateway",
            "options": {"baseURL": grant.proxy_base_url, "apiKey": "{env:ORA_MODEL_ACCESS_TOKEN}"},
            "models": {model_id: {
                "name": grant.model.name,
                "limit": {"context": grant.model.context_window, "output": grant.model.max_tokens},
            }},
        }},
    });
    let mut variables = BTreeMap::from([
        ("OPENCODE_CONFIG_CONTENT".to_string(), config.to_string()),
        ("ORA_MODEL_ACCESS_TOKEN".to_string(), grant.token.clone()),
        (
            "NODE_EXTRA_CA_CERTS".to_string(),
            ca.to_string_lossy().into_owned(),
        ),
        (
            "HOME".to_string(),
            home.path().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CONFIG_HOME".to_string(),
            home.path().join("config").to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME".to_string(),
            home.path().join("cache").to_string_lossy().into_owned(),
        ),
        (
            "XDG_DATA_HOME".to_string(),
            home.path().join("data").to_string_lossy().into_owned(),
        ),
        (
            "XDG_STATE_HOME".to_string(),
            home.path().join("state").to_string_lossy().into_owned(),
        ),
        (
            "OPENCODE_DISABLE_AUTOUPDATE".to_string(),
            "true".to_string(),
        ),
        (
            "OPENCODE_DISABLE_MODELS_FETCH".to_string(),
            "true".to_string(),
        ),
    ]);
    // Disallow discovery of a user's global config and credentials on this management host.
    variables.insert(
        "OPENCODE_CONFIG_DIR".to_string(),
        home.path().join("config").to_string_lossy().into_owned(),
    );
    Ok((variables, home))
}
