//! The Controller's client of the Substrate effects interface (`GET/PUT /effects/{effectId}`).
//! Cloud plans every effect and its request; this client only carries the plan to the Substrate
//! under the effect ID Cloud allocated, which is the Substrate's idempotency key, and translates
//! what the Substrate reports back into the contract's evidence. It never rewrites an ID or a
//! request, and it never concludes that an effect is absent because a call timed out.
use crate::{Error, SubstrateConfig};
use ora_controller_proto::v1::{self as proto, effect_evidence::Evidence, effect_request::Request};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::time::Duration;

/// What the Substrate reported for one effect, in the contract's terms.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Observation {
    /// The Substrate is still working on it, or a crash left its outcome to be reconciled.
    Running { external_id: String },
    /// Done; the evidence is what Cloud checks before advancing.
    Succeeded {
        external_id: String,
        evidence: proto::EffectEvidence,
    },
    /// The Substrate gave up with a bounded reason; the same effect may be retried later.
    Failed {
        external_id: String,
        failure: Option<String>,
    },
}

/// Why an effect produced no observation.
#[derive(Debug, thiserror::Error)]
pub(super) enum SubstrateError {
    /// No answer, or no journal entry even after the request may have been sent: retry the same
    /// effect later. Never proof that nothing happened.
    #[error("substrate unreachable: {0}")]
    Unreachable(String),
    /// The Substrate refused the request itself (unknown kind, invalid scope, a different request
    /// under the same ID); sending it again cannot succeed.
    #[error("substrate rejected the effect: {0}")]
    Rejected(String),
}

/// One journal entry as the Substrate returns it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    external_id: String,
    state: String,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: Option<String>,
}

/// A client bound to one Substrate. Cheap to clone; clones share the connection pool.
#[derive(Clone)]
pub(super) struct Substrate {
    client: reqwest::Client,
    base: String,
}

impl Substrate {
    /// Validates the configured base URL and deadline; nothing is contacted.
    pub(super) fn new(config: &SubstrateConfig) -> Result<Self, Error> {
        let url = reqwest::Url::parse(&config.effects_url).map_err(|error| {
            Error::Configuration(format!("invalid substrate.effects_url: {error}"))
        })?;
        if !matches!(url.scheme(), "http" | "https") || config.request_timeout_ms == 0 {
            return Err(Error::Configuration(
                "substrate.effects_url must be http(s) and request_timeout_ms nonzero".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.request_timeout_ms))
            .build()
            .map_err(|error| Error::Configuration(format!("substrate client: {error}")))?;
        Ok(Self {
            client,
            base: config.effects_url.trim_end_matches('/').to_owned(),
        })
    }

    fn url(&self, effect: &str) -> String {
        format!("{}/effects/{effect}", self.base)
    }

    /// Reads the Substrate's journal entry for one effect; `None` means the Substrate knows for
    /// certain that it never received it.
    pub(super) async fn get(
        &self,
        effect: &proto::Effect,
    ) -> Result<Option<Observation>, SubstrateError> {
        let response = self
            .client
            .get(self.url(&effect.id))
            .send()
            .await
            .map_err(|error| SubstrateError::Unreachable(error.to_string()))?;
        match response.status().as_u16() {
            404 => Ok(None),
            200 => {
                let entry = response
                    .json::<Entry>()
                    .await
                    .map_err(|error| SubstrateError::Unreachable(error.to_string()))?;
                observation(effect.kind(), entry).map(Some)
            }
            status => Err(SubstrateError::Unreachable(format!(
                "GET answered {status}"
            ))),
        }
    }

    /// Carries one effect to the Substrate: an effect it already completed is only read back, any
    /// other is sent with Cloud's exact request. When the send is not answered, the same ID is
    /// queried, because the Substrate may have journaled and executed it.
    pub(super) async fn execute(
        &self,
        effect: &proto::Effect,
    ) -> Result<Observation, SubstrateError> {
        if let Some(observed @ Observation::Succeeded { .. }) = self.get(effect).await? {
            return Ok(observed);
        }
        let body = request(effect)?;
        let sent = self
            .client
            .put(self.url(&effect.id))
            .json(&body)
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(error) => {
                ora_logging::ora_warn!(effect_id = %effect.id, error = %error, "Substrate did not answer an effect; querying it by the same ID");
                return self.get(effect).await?.ok_or_else(|| {
                    SubstrateError::Unreachable(format!("no answer and no journal entry: {error}"))
                });
            }
        };
        match response.status().as_u16() {
            // A failed effect is still a journal entry; its body says why.
            200 | 503 => {
                let entry = response
                    .json::<Entry>()
                    .await
                    .map_err(|error| SubstrateError::Unreachable(error.to_string()))?;
                observation(effect.kind(), entry)
            }
            status @ (400 | 404 | 405 | 409 | 422) => {
                let body = response.text().await.unwrap_or_default();
                Err(SubstrateError::Rejected(format!(
                    "PUT answered {status}: {body}"
                )))
            }
            status => {
                ora_logging::ora_warn!(effect_id = %effect.id, status, "Substrate failed an effect call; querying it by the same ID");
                self.get(effect).await?.ok_or_else(|| {
                    SubstrateError::Unreachable(format!(
                        "PUT answered {status} and nothing was journaled"
                    ))
                })
            }
        }
    }
}

/// The JSON request Cloud planned, rebuilt from the contract's typed form. Cloud's JSON uses the
/// same field names, so the Substrate sees the plan unchanged.
fn request(effect: &proto::Effect) -> Result<Value, SubstrateError> {
    let request = effect
        .request
        .as_ref()
        .and_then(|request| request.request.as_ref())
        .ok_or_else(|| SubstrateError::Rejected("effect carries no request".into()))?;
    Ok(match request {
        Request::SandboxEnsure(ensure) => json!({
            "kind": "sandbox_ensure",
            "projectId": ensure.project_id,
            "workspaceId": ensure.workspace_id,
        }),
        Request::SandboxTerminate(terminate) => json!({
            "kind": "sandbox_terminate",
            "projectId": terminate.project_id,
            "workspaceId": terminate.workspace_id,
            "sandboxInstanceId": terminate.sandbox_instance_id,
        }),
        Request::WorkspaceDataDelete(delete) => json!({
            "kind": "workspace_data_delete",
            "projectId": delete.project_id,
            "workspaceId": delete.workspace_id,
        }),
        Request::PluginEnsure(ensure) => {
            let artifact = |artifact: &proto::PluginArtifact| {
                let mut object = Map::new();
                if let Some(target) = &artifact.target {
                    object.insert("target".into(), target.clone().into());
                }
                object.insert("url".into(), artifact.url.clone().into());
                object.insert("sha256".into(), artifact.sha256.clone().into());
                Value::Object(object)
            };
            let mut object = Map::new();
            object.insert("kind".into(), "plugin_ensure".into());
            object.insert("projectId".into(), ensure.project_id.clone().into());
            object.insert("workspaceId".into(), ensure.workspace_id.clone().into());
            object.insert("pluginId".into(), ensure.plugin_id.clone().into());
            object.insert("version".into(), ensure.version.clone().into());
            if let Some(universal) = &ensure.universal {
                object.insert("universal".into(), artifact(universal));
            }
            if !ensure.targets.is_empty() {
                object.insert(
                    "targets".into(),
                    ensure.targets.iter().map(artifact).collect(),
                );
            }
            Value::Object(object)
        }
        Request::PluginDelete(delete) => json!({
            "kind": "plugin_delete",
            "projectId": delete.project_id,
            "workspaceId": delete.workspace_id,
            "pluginId": delete.plugin_id,
            "version": delete.version,
        }),
    })
}

/// Reads one journal entry as an observation of an effect of `kind`. Success without the evidence
/// its kind requires is not success: it stays running so Cloud never records unproven evidence.
fn observation(kind: proto::EffectKind, entry: Entry) -> Result<Observation, SubstrateError> {
    let external_id = entry.external_id;
    if external_id.is_empty() {
        return Err(SubstrateError::Rejected(
            "journal entry has no externalId".into(),
        ));
    }
    match entry.state.as_str() {
        "succeeded" => match evidence(kind, &entry.result) {
            Some(evidence) => Ok(Observation::Succeeded {
                external_id,
                evidence: proto::EffectEvidence {
                    evidence: Some(evidence),
                },
            }),
            None => Err(SubstrateError::Rejected(format!(
                "succeeded without {} evidence",
                kind.as_str_name()
            ))),
        },
        "failed" => Ok(Observation::Failed {
            external_id,
            failure: entry.error,
        }),
        _ => Ok(Observation::Running { external_id }),
    }
}

/// The success evidence the contract names for each kind, taken from the Substrate's result.
fn evidence(kind: proto::EffectKind, result: &Value) -> Option<Evidence> {
    let text = |key: &str| result.get(key).and_then(Value::as_str).map(str::to_owned);
    let flag = |key: &str| result.get(key).and_then(Value::as_bool) == Some(true);
    match kind {
        proto::EffectKind::SandboxEnsure => Some(Evidence::SandboxEnsured(proto::SandboxEnsured {
            sandbox_instance_id: text("sandboxInstanceId")?,
            node_id: text("nodeId")?,
        })),
        proto::EffectKind::SandboxTerminate => {
            flag("terminated").then_some(Evidence::SandboxTerminated(proto::SandboxTerminated {}))
        }
        proto::EffectKind::WorkspaceDataDelete => flag("removed").then_some(
            Evidence::WorkspaceDataDeleted(proto::WorkspaceDataDeleted {}),
        ),
        proto::EffectKind::PluginEnsure => flag("installed").then(|| {
            Evidence::PluginInstalled(proto::PluginInstalled {
                version: text("version").unwrap_or_default(),
            })
        }),
        proto::EffectKind::PluginDelete => {
            flag("removed").then_some(Evidence::PluginRemoved(proto::PluginRemoved {}))
        }
        proto::EffectKind::Unspecified => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::{Path, State},
        http::{Method, StatusCode},
        routing::any,
    };
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, Mutex};

    /// How the fake answers each PUT.
    #[derive(Clone, Copy)]
    enum Put {
        /// Journal the entry as succeeded and answer it.
        Answer,
        /// Journal the entry as succeeded, then never answer, as a lost response looks.
        Hang,
    }

    #[derive(Clone)]
    struct Fake {
        calls: Arc<Mutex<Vec<(Method, String)>>>,
        journal: Arc<Mutex<Option<Value>>>,
        put: Put,
    }

    async fn effects(
        State(fake): State<Fake>,
        method: Method,
        Path(id): Path<String>,
    ) -> (StatusCode, String) {
        fake.calls
            .lock()
            .unwrap()
            .push((method.clone(), id.clone()));
        if method == Method::PUT {
            let entry = json!({
                "id": id, "externalId": id, "state": "succeeded", "request": {},
                "result": { "sandboxInstanceId": id, "nodeId": "workspace-w" },
            });
            *fake.journal.lock().unwrap() = Some(entry.clone());
            if let Put::Hang = fake.put {
                std::future::pending::<()>().await;
            }
            return (StatusCode::OK, entry.to_string());
        }
        match fake.journal.lock().unwrap().clone() {
            Some(entry) => (StatusCode::OK, entry.to_string()),
            None => (StatusCode::NOT_FOUND, String::new()),
        }
    }

    /// Runs `test` under the scoped TRACE subscriber, since the client logs lost responses.
    fn traced(test: impl Future<Output = ()>) {
        ora_logging::with_trace_logging(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(test);
        });
    }

    async fn serve(put: Put) -> (Substrate, Fake) {
        let fake = Fake {
            calls: Arc::default(),
            journal: Arc::default(),
            put,
        };
        let app = Router::new()
            .route("/effects/{id}", any(effects))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let substrate = Substrate::new(&SubstrateConfig {
            effects_url: format!("http://{address}"),
            router_url: "ws://127.0.0.1:1/ora-node/v1".into(),
            atespace: "local".into(),
            request_timeout_ms: 200,
        })
        .unwrap();
        (substrate, fake)
    }

    fn ensure(id: &str) -> proto::Effect {
        proto::Effect {
            id: id.into(),
            kind: proto::EffectKind::SandboxEnsure as i32,
            request: Some(proto::EffectRequest {
                request: Some(Request::SandboxEnsure(proto::SandboxEnsureRequest {
                    project_id: "p".into(),
                    workspace_id: "w".into(),
                })),
            }),
            ..proto::Effect::default()
        }
    }

    fn ensured(id: &str) -> Observation {
        Observation::Succeeded {
            external_id: id.into(),
            evidence: proto::EffectEvidence {
                evidence: Some(Evidence::SandboxEnsured(proto::SandboxEnsured {
                    sandbox_instance_id: id.into(),
                    node_id: "workspace-w".into(),
                })),
            },
        }
    }

    /// An unanswered send is followed by a query of the same effect ID, whose journal entry is
    /// the outcome; the effect is never re-sent under another identity.
    ///
    /// Spec: specs/test-cases/controller/node-management/workspace-sandbox-driving.md#dynamic-node-targets-follow-cloud-sandboxes-only
    #[test]
    fn a_timed_out_effect_is_queried_by_the_same_id() {
        traced(async {
            let (substrate, fake) = serve(Put::Hang).await;
            let observed = substrate.execute(&ensure("e1")).await.unwrap();
            assert_eq!(observed, ensured("e1"));
            assert_eq!(
                fake.calls.lock().unwrap().clone(),
                vec![
                    (Method::GET, "e1".to_owned()),
                    (Method::PUT, "e1".to_owned()),
                    (Method::GET, "e1".to_owned()),
                ]
            );
        });
    }

    /// An effect the Substrate already completed is read back, not sent again.
    #[test]
    fn a_completed_effect_is_not_sent_again() {
        traced(async {
            let (substrate, fake) = serve(Put::Answer).await;
            assert_eq!(
                substrate.execute(&ensure("e2")).await.unwrap(),
                ensured("e2")
            );
            assert_eq!(
                substrate.execute(&ensure("e2")).await.unwrap(),
                ensured("e2")
            );
            assert_eq!(
                fake.calls.lock().unwrap().clone(),
                vec![
                    (Method::GET, "e2".to_owned()),
                    (Method::PUT, "e2".to_owned()),
                    (Method::GET, "e2".to_owned()),
                ]
            );
        });
    }

    /// Cloud's typed request is sent with Cloud's JSON field names, and each kind's evidence is
    /// required before success is reported.
    #[test]
    fn requests_and_evidence_follow_the_contract_shapes() {
        let terminate = proto::Effect {
            id: "t".into(),
            kind: proto::EffectKind::SandboxTerminate as i32,
            request: Some(proto::EffectRequest {
                request: Some(Request::SandboxTerminate(proto::SandboxTerminateRequest {
                    project_id: "p".into(),
                    workspace_id: "w".into(),
                    sandbox_instance_id: "s".into(),
                })),
            }),
            ..proto::Effect::default()
        };
        assert_eq!(
            request(&terminate).unwrap(),
            json!({ "kind": "sandbox_terminate", "projectId": "p", "workspaceId": "w", "sandboxInstanceId": "s" })
        );
        let entry = |state: &str, result: Value| Entry {
            external_id: "x".into(),
            state: state.into(),
            result,
            error: Some("docker_failure".into()),
        };
        assert!(matches!(
            observation(
                proto::EffectKind::SandboxTerminate,
                entry("succeeded", json!({}))
            ),
            Err(SubstrateError::Rejected(_))
        ));
        assert_eq!(
            observation(
                proto::EffectKind::WorkspaceDataDelete,
                entry("succeeded", json!({ "removed": true }))
            )
            .unwrap(),
            Observation::Succeeded {
                external_id: "x".into(),
                evidence: proto::EffectEvidence {
                    evidence: Some(Evidence::WorkspaceDataDeleted(
                        proto::WorkspaceDataDeleted {}
                    )),
                },
            }
        );
        assert_eq!(
            observation(proto::EffectKind::SandboxEnsure, entry("failed", json!({}))).unwrap(),
            Observation::Failed {
                external_id: "x".into(),
                failure: Some("docker_failure".into()),
            }
        );
        assert_eq!(
            observation(
                proto::EffectKind::SandboxEnsure,
                entry("running", json!({}))
            )
            .unwrap(),
            Observation::Running {
                external_id: "x".into(),
            }
        );
    }
}
