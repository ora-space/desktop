//! Short-lived platform model grants owned by one live agent session, never by its ledger.

mod opencode;
#[cfg(test)]
pub(crate) mod tests;

use chrono::{DateTime, FixedOffset};
use ora_node_protocol::{ExecutionId, ModelBindingId};
use reqwest::{Certificate, Client, Identity, Url};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

/// Dedicated model-access client TLS supplied by deployment, separate from Node server TLS.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProxyConfig {
    pub gateway_url: String,
    pub ca_cert: PathBuf,
    pub client_cert: PathBuf,
    pub client_key: PathBuf,
}

/// Errors expose bounded codes only: HTTP bodies and credentials are never error messages.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct ModelAccessError(pub(super) &'static str);

/// Only an authenticated ending intent may wait for the durable EndSession command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelAccessStatus {
    Active,
    Ending,
    Revoked,
}

#[derive(Clone)]
struct ModelClient {
    http: Client,
    gateway: Url,
    config: ModelProxyConfig,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Protocol {
    OpenaiCompletions,
    AnthropicMessages,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Model {
    id: String,
    name: String,
    context_window: u64,
    max_tokens: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Grant {
    grant_id: String,
    token: String,
    expires_at: DateTime<FixedOffset>,
    protocol: Protocol,
    proxy_base_url: String,
    model: Model,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Renewal {
    expires_at: DateTime<FixedOffset>,
}

impl ModelProxyConfig {
    /// Rejects invalid deployment material before advertising model access to a Controller.
    pub(crate) fn validate(&self) -> Result<(), ModelAccessError> {
        ModelClient::open(self.clone()).map(drop)
    }
}

impl ModelClient {
    /// Opens a private mTLS client; redirects could disclose a grant to another service.
    fn open(config: ModelProxyConfig) -> Result<Self, ModelAccessError> {
        let gateway = Url::parse(&config.gateway_url)
            .map_err(|_| ModelAccessError("model_proxy_configuration_invalid"))?;
        if gateway.scheme() != "https"
            || gateway.host_str().is_none()
            || !gateway.username().is_empty()
            || gateway.password().is_some()
            || gateway.query().is_some()
            || gateway.fragment().is_some()
            || gateway.path() != "/"
            || [&config.ca_cert, &config.client_cert, &config.client_key]
                .into_iter()
                .any(|path| !path.is_absolute())
        {
            return Err(ModelAccessError("model_proxy_configuration_invalid"));
        }
        let ca = std::fs::read(&config.ca_cert)
            .map_err(|_| ModelAccessError("model_proxy_tls_invalid"))?;
        let mut identity = std::fs::read(&config.client_cert)
            .map_err(|_| ModelAccessError("model_proxy_tls_invalid"))?;
        identity.push(b'\n');
        identity.extend(
            std::fs::read(&config.client_key)
                .map_err(|_| ModelAccessError("model_proxy_tls_invalid"))?,
        );
        let http = Client::builder()
            .https_only(/*enabled*/ true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .tls_built_in_root_certs(/*tls_built_in_root_certs*/ false)
            .add_root_certificate(
                Certificate::from_pem(&ca)
                    .map_err(|_| ModelAccessError("model_proxy_tls_invalid"))?,
            )
            .identity(
                Identity::from_pem(&identity)
                    .map_err(|_| ModelAccessError("model_proxy_tls_invalid"))?,
            )
            .timeout(Duration::from_secs(/*secs*/ 15))
            .build()
            .map_err(|_| ModelAccessError("model_proxy_tls_invalid"))?;
        Ok(Self {
            http,
            gateway,
            config,
        })
    }

    /// Calls only the fixed grant surface over the provisioned service identity.
    fn endpoint(&self, suffix: &str) -> Result<Url, ModelAccessError> {
        self.gateway
            .join(&format!("internal/v1/model-grants{suffix}"))
            .map_err(|_| ModelAccessError("model_proxy_configuration_invalid"))
    }

    /// Obtains the model frozen for this execution; scope comes from the mTLS certificate.
    async fn grant(
        &self,
        binding: &ModelBindingId,
        execution: &ExecutionId,
    ) -> Result<Grant, ModelAccessError> {
        let response = self.http.post(self.endpoint("")?)
            .json(&serde_json::json!({"bindingId": binding.as_str(), "executionId": execution.as_str()}))
            .send().await.map_err(|_| ModelAccessError("model_proxy_unavailable"))?;
        if !response.status().is_success() {
            return Err(denial(response, "model_access_denied").await);
        }
        let grant: Grant = response
            .json()
            .await
            .map_err(|_| ModelAccessError("model_grant_invalid"))?;
        let data = match Url::parse(&grant.proxy_base_url) {
            Ok(data) => data,
            Err(_) => {
                if valid_grant_id(&grant.grant_id) {
                    self.revoke(&grant.grant_id, execution).await;
                }
                return Err(ModelAccessError("model_grant_invalid"));
            }
        };
        // The token is sent only to this gateway's data listener, never to a returned third party.
        if data.scheme() != "https"
            || data.host_str() != self.gateway.host_str()
            || !data.username().is_empty()
            || data.password().is_some()
            || data.query().is_some()
            || data.fragment().is_some()
            || data.path()
                != match grant.protocol {
                    Protocol::OpenaiCompletions => "/runtime/openai/v1",
                    Protocol::AnthropicMessages => "/runtime/anthropic/v1",
                }
            || grant.token.is_empty()
            || grant.model.id.is_empty()
            || !valid_grant_id(&grant.grant_id)
            || renewal_delay(grant.expires_at).is_err()
        {
            if valid_grant_id(&grant.grant_id) {
                self.revoke(&grant.grant_id, execution).await;
            }
            return Err(ModelAccessError("model_grant_invalid"));
        }
        Ok(grant)
    }

    /// Extends the existing digest; the CLI's environment keeps the same temporary token.
    async fn renew(
        &self,
        grant: &str,
        execution: &ExecutionId,
    ) -> Result<DateTime<FixedOffset>, ModelAccessError> {
        let response = self
            .http
            .post(self.endpoint(&format!("/{grant}/renew"))?)
            .json(&serde_json::json!({"executionId": execution.as_str()}))
            .send()
            .await
            .map_err(|_| ModelAccessError("model_proxy_unavailable"))?;
        if !response.status().is_success() {
            return Err(denial(response, "model_access_revoked").await);
        }
        response
            .json::<Renewal>()
            .await
            .map(|value| value.expires_at)
            .map_err(|_| ModelAccessError("model_grant_invalid"))
    }

    /// Makes completed and cancelled sessions unable to generate another model request.
    async fn revoke(&self, grant: &str, execution: &ExecutionId) {
        if let Ok(endpoint) = self.endpoint(&format!("/{grant}")) {
            let _ = self
                .http
                .delete(endpoint)
                .json(&serde_json::json!({"executionId": execution.as_str()}))
                .send()
                .await;
        }
    }
}

/// Only a bounded path segment may enter the fixed grant route; it is not an arbitrary URL.
fn valid_grant_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

/// A renewable grant and isolated OpenCode state whose owner is exactly one live conversation.
pub(super) struct ModelAccess {
    pub(super) environment: BTreeMap<String, String>,
    _home: tempfile::TempDir,
    client: ModelClient,
    grant: String,
    execution: ExecutionId,
    renewal: JoinHandle<()>,
    pub(super) status: watch::Receiver<ModelAccessStatus>,
    closed: bool,
}

impl ModelAccess {
    /// Creates ephemeral configuration outside the checkout, then renews its in-memory grant.
    pub(super) async fn open(
        config: ModelProxyConfig,
        binding: &ModelBindingId,
        execution: &ExecutionId,
        node_home: &std::path::Path,
    ) -> Result<Self, ModelAccessError> {
        let client = ModelClient::open(config)?;
        let grant = client.grant(binding, execution).await?;
        let setup = opencode::environment(&grant, &client.config.ca_cert, node_home);
        let (environment, home) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                client.revoke(&grant.grant_id, execution).await;
                return Err(error);
            }
        };
        let (failure, status) = watch::channel(ModelAccessStatus::Active);
        let renewal_client = client.clone();
        let renewal_grant = grant.grant_id.clone();
        let renewal_execution = execution.clone();
        let mut expires = grant.expires_at;
        let renewal = tokio::spawn(async move {
            let mut status = ModelAccessStatus::Revoked;
            while let Ok(delay) = renewal_delay(expires) {
                tokio::time::sleep(delay).await;
                match renewal_client
                    .renew(&renewal_grant, &renewal_execution)
                    .await
                {
                    Ok(renewed) => expires = renewed,
                    Err(error) => {
                        if error.0 == "model_session_ending" {
                            status = ModelAccessStatus::Ending;
                        }
                        break;
                    }
                }
            }
            failure.send_replace(status);
        });
        Ok(Self {
            environment,
            _home: home,
            client,
            grant: grant.grant_id,
            execution: execution.clone(),
            renewal,
            status,
            closed: false,
        })
    }

    /// Stops renewal before revocation; cleanup never extends an already finished run.
    pub(super) async fn close(&mut self) {
        if self.closed {
            return;
        }
        self.renewal.abort();
        self.client.revoke(&self.grant, &self.execution).await;
        self.closed = true;
    }
}

/// Reads only a bounded safe fault code; all other upstream error content is discarded.
async fn denial(mut response: reqwest::Response, fallback: &'static str) -> ModelAccessError {
    let ending_status = response.status() == reqwest::StatusCode::FORBIDDEN;
    #[derive(Deserialize)]
    struct FaultCode {
        code: String,
    }
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if body.len() + chunk.len() > 16 * 1024 {
            return ModelAccessError(fallback);
        }
        body.extend_from_slice(&chunk);
    }
    if ending_status
        && serde_json::from_slice::<FaultCode>(&body)
            .is_ok_and(|fault| fault.code == "model_session_ending")
    {
        ModelAccessError("model_session_ending")
    } else {
        ModelAccessError(fallback)
    }
}

impl Drop for ModelAccess {
    /// Task cancellation still stops renewal and revokes best effort without retaining a token.
    fn drop(&mut self) {
        self.renewal.abort();
        if !self.closed
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let client = self.client.clone();
            let grant = self.grant.clone();
            let execution = self.execution.clone();
            runtime.spawn(async move {
                client.revoke(&grant, &execution).await;
            });
        }
    }
}

/// Renews one minute before expiry; short test grants renew halfway through their remaining life.
fn renewal_delay(expires: DateTime<FixedOffset>) -> Result<Duration, ModelAccessError> {
    let millis = i128::from(expires.timestamp_millis())
        - ora_logging::clock::now_local().unix_timestamp_nanos() / 1_000_000;
    let remaining = Duration::from_millis(
        u64::try_from(millis).map_err(|_| ModelAccessError("model_access_expired"))?,
    );
    if remaining.is_zero() || remaining > Duration::from_secs(/*secs*/ 16 * 60) {
        return Err(ModelAccessError("model_grant_invalid"));
    }
    Ok(remaining
        .saturating_sub(Duration::from_secs(/*secs*/ 60))
        .max(remaining / 2))
}
