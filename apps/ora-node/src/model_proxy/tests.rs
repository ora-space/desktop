#![allow(clippy::unwrap_used)]
use super::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
#[path = "../../test-support/model_gateway.rs"]
mod test_gateway;
pub(crate) use test_gateway::Gateway;
#[tokio::test]
async fn renewable_access_keeps_token_in_memory_and_revokes_at_end() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    let mut access = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await
    .unwrap();
    let home = PathBuf::from(&access.environment["HOME"]);
    assert_eq!(
        access
            .environment
            .get("OPENCODE_DISABLE_PROJECT_CONFIG")
            .map(String::as_str),
        Some("true")
    );
    let config: Value =
        serde_json::from_str(&access.environment["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["model"], "ora-model/team/model");
    assert_eq!(
        config["provider"]["ora-model"]["options"]["apiKey"],
        "{env:ORA_MODEL_ACCESS_TOKEN}"
    );
    assert!(!config.to_string().contains("synthetic-temporary-token"));
    tokio::time::timeout(Duration::from_secs(/*secs*/ 4), async {
        while gateway.requests.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(/*millis*/ 10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        access.environment["ORA_MODEL_ACCESS_TOKEN"],
        "synthetic-temporary-token"
    );
    access.close().await;
    drop(access);
    assert!(!home.exists());
    assert_eq!(
        *gateway.requests.lock().unwrap(),
        vec![
            (
                "POST /internal/v1/model-grants HTTP/1.1".to_string(),
                json!({"bindingId":"binding-1","executionId":"execution-1"})
            ),
            (
                "POST /internal/v1/model-grants/grant-1/renew HTTP/1.1".to_string(),
                json!({"executionId":"execution-1"})
            ),
            (
                "DELETE /internal/v1/model-grants/grant-1 HTTP/1.1".to_string(),
                json!({"executionId":"execution-1"})
            ),
        ]
    );
}

#[tokio::test]
async fn foreign_data_endpoint_is_rejected_and_grant_revoked() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    gateway
        .behavior
        .foreign_endpoint
        .store(true, Ordering::SeqCst);
    let result = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await;
    let Err(error) = result else {
        panic!("foreign endpoint must not receive a token")
    };
    assert_eq!(error.to_string(), "model_grant_invalid");
    assert_eq!(
        gateway
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(route, _)| route.clone())
            .collect::<Vec<_>>(),
        vec![
            "POST /internal/v1/model-grants HTTP/1.1",
            "DELETE /internal/v1/model-grants/grant-1 HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn denied_renewal_notifies_the_conversation_to_stop() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    gateway.behavior.deny_renewal.store(true, Ordering::SeqCst);
    let mut access = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await
    .unwrap();
    let status = tokio::time::timeout(
        Duration::from_secs(/*secs*/ 4),
        access
            .status
            .wait_for(|status| *status != ModelAccessStatus::Active),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(*status, ModelAccessStatus::Revoked);
    drop(status);
    access.close().await;
    assert_eq!(gateway.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn close_cancels_the_scheduled_renewal() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    let mut access = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &ExecutionId::new("execution-1"),
        root.path(),
    )
    .await
    .unwrap();
    access.close().await;
    // This crosses the fixture grant's expiry, so an unowned timer would have renewed by now.
    tokio::time::sleep(Duration::from_millis(/*millis*/ 2200)).await;
    assert_eq!(gateway.requests.lock().unwrap().len(), 2);
}

#[test]
fn anthropic_configuration_preserves_model_ids_and_isolates_cli_storage() {
    let root = tempfile::tempdir().unwrap();
    let grant = Grant {
        grant_id: "grant-1".into(),
        token: "synthetic-token".into(),
        expires_at: chrono::Local::now().fixed_offset(),
        protocol: Protocol::AnthropicMessages,
        proxy_base_url: "https://gateway/runtime/anthropic/v1".into(),
        model: Model {
            id: "team/model".into(),
            name: "Model".into(),
            context_window: 200000,
            max_tokens: 8192,
        },
    };
    let (environment, home) =
        opencode::environment(&grant, &root.path().join("ca.pem"), root.path()).unwrap();
    let config: Value = serde_json::from_str(&environment["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["provider"]["ora-model"]["npm"], "@ai-sdk/anthropic");
    assert_eq!(
        config["provider"]["ora-model"]["models"]["team/model"],
        json!({"name":"Model","limit":{"context":200000,"output":8192}})
    );
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ] {
        assert!(PathBuf::from(&environment[key]).starts_with(home.path()));
    }
    assert!(!config.to_string().contains("synthetic-token"));
}

#[test]
fn expired_and_excessive_grant_lifetimes_are_rejected() {
    ora_logging::initialize_test_clock();
    assert!(
        renewal_delay(chrono::Local::now().fixed_offset() - chrono::Duration::seconds(1)).is_err()
    );
    assert!(
        renewal_delay(chrono::Local::now().fixed_offset() + chrono::Duration::hours(1)).is_err()
    );
}

/// A CA file published to a workload must not contain a combined private-key PEM block.
#[test]
fn public_ca_material_rejects_private_key_blocks_before_grant_creation() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    let mut combined = std::fs::read(&gateway.config.ca_cert).unwrap();
    combined.extend(std::fs::read(&gateway.config.client_key).unwrap());
    std::fs::write(&gateway.config.ca_cert, combined).unwrap();
    let error = gateway.config.validate().unwrap_err();
    assert_eq!(error.to_string(), "model_proxy_tls_invalid");
    assert!(gateway.requests.lock().unwrap().is_empty());
}

#[test]
fn runtime_state_refuses_existing_files_and_symlinks_without_changing_them() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let grant = Grant {
        grant_id: "grant-1".into(),
        token: "synthetic-token".into(),
        expires_at: chrono::Local::now().fixed_offset(),
        protocol: Protocol::OpenaiCompletions,
        proxy_base_url: "https://gateway/runtime/openai/v1".into(),
        model: Model {
            id: "model".into(),
            name: "Model".into(),
            context_window: 32768,
            max_tokens: 1024,
        },
    };
    let runtime = root.path().join("model-runtime");
    std::fs::write(&runtime, b"existing user data").unwrap();
    assert!(opencode::environment(&grant, &root.path().join("ca.pem"), root.path()).is_err());
    assert_eq!(std::fs::read(&runtime).unwrap(), b"existing user data");
    std::fs::remove_file(&runtime).unwrap();
    symlink(checkout.path(), &runtime).unwrap();
    assert!(opencode::environment(&grant, &root.path().join("ca.pem"), root.path()).is_err());
    assert_eq!(std::fs::read_dir(checkout.path()).unwrap().count(), 0);
}
