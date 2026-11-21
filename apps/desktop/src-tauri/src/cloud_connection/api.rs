//! Calls to the Cloud server's desktop-login routes and the account check, with one rule for what a
//! failure means for the stored token.

use std::future::Future;
use std::time::Duration;

use ora_contracts::CloudAccount;
use ora_utils::http::{ProxyConfig, credential_client};
use serde::Deserialize;
use url::Url;

use super::secret::SessionToken;
use super::server::CloudServer;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// What a failed Cloud call says about the session.
///
/// Only `Rejected` may end a session: a network error or a server fault says nothing about the
/// token, and deleting it then would sign people out whenever the server restarts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CallError {
    /// 401: the token (or code) is no longer accepted.
    Rejected,
    /// The server could not be reached, timed out, answered 5xx or tried to redirect (the client
    /// never follows a redirect, so the token cannot reach another origin).
    Unreachable(String),
    /// Any other answer, such as 400 for a spent code or a body that is not the contract's.
    Request(String),
}

/// A token just issued by the server, with the account it belongs to.
pub(crate) struct IssuedToken {
    pub(crate) token: SessionToken,
    pub(crate) account: CloudAccount,
}

/// The code exchange inputs; they must match what the server saw at approval.
pub(crate) struct CodeExchange<'a> {
    pub(crate) code: &'a str,
    pub(crate) verifier: &'a str,
    pub(crate) redirect_uri: &'a str,
}

/// The Cloud routes the connection uses. Implementations send to `server` and nowhere else.
pub(crate) trait CloudApi: Send + Sync + 'static {
    /// `POST /auth/desktop/token`.
    fn exchange(
        &self,
        server: &CloudServer,
        exchange: CodeExchange<'_>,
    ) -> impl Future<Output = Result<IssuedToken, CallError>> + Send;
    /// `GET /api/v1/me` with the token.
    fn me(
        &self,
        server: &CloudServer,
        token: &SessionToken,
    ) -> impl Future<Output = Result<CloudAccount, CallError>> + Send;
    /// `POST /auth/desktop/logout` with the token.
    fn logout(
        &self,
        server: &CloudServer,
        token: &SessionToken,
    ) -> impl Future<Output = Result<(), CallError>> + Send;
}

/// The production client: platform certificate verification, environment proxy, no redirects.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HttpCloudApi;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserBody {
    id: String,
    display_name: String,
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
    user: UserBody,
}

impl HttpCloudApi {
    /// Builds a client for one call. The proxy follows the environment (with loopback and
    /// `NO_PROXY` bypassed) rather than the marketplace proxy setting, which Settings describes as
    /// applying only to marketplace sources that opt into it.
    fn client(server: &CloudServer) -> Result<reqwest::Client, CallError> {
        let url =
            Url::parse(server.origin()).map_err(|error| CallError::Request(error.to_string()))?;
        credential_client(
            &ProxyConfig::default(),
            &url,
            CONNECT_TIMEOUT,
            REQUEST_TIMEOUT,
        )
        .map_err(|error| {
            CallError::Unreachable(format!("cannot build the Cloud client: {error:?}"))
        })
    }

    /// Sends a request and maps its outcome to the session rule above.
    async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response, CallError> {
        let response = request
            .send()
            .await
            .map_err(|error| CallError::Unreachable(error.without_url().to_string()))?;
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            Err(CallError::Rejected)
        } else if status.is_server_error() || status.is_redirection() {
            Err(CallError::Unreachable(format!("server answered {status}")))
        } else {
            Err(CallError::Request(format!("server answered {status}")))
        }
    }

    /// Reads a contract body; a body that does not parse is the server's fault, not the token's.
    async fn body<T: serde::de::DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, CallError> {
        let bytes = response
            .bytes()
            .await
            .map_err(|error| CallError::Unreachable(error.without_url().to_string()))?;
        serde_json::from_slice(&bytes).map_err(|error| CallError::Request(error.to_string()))
    }
}

impl CloudApi for HttpCloudApi {
    /// Exchanges the code; the JSON names follow the server's candidate contract.
    async fn exchange(
        &self,
        server: &CloudServer,
        exchange: CodeExchange<'_>,
    ) -> Result<IssuedToken, CallError> {
        let client = Self::client(server)?;
        let request =
            client
                .post(server.endpoint("/auth/desktop/token"))
                .json(&serde_json::json!({
                    "code": exchange.code,
                    "codeVerifier": exchange.verifier,
                    "redirectUri": exchange.redirect_uri,
                }));
        let body: TokenBody = Self::body(Self::send(request).await?).await?;
        Ok(IssuedToken {
            token: SessionToken::new(body.token),
            account: CloudAccount {
                id: body.user.id,
                display_name: body.user.display_name,
            },
        })
    }

    /// Asks who the token belongs to; this is also how revocation becomes visible.
    async fn me(
        &self,
        server: &CloudServer,
        token: &SessionToken,
    ) -> Result<CloudAccount, CallError> {
        let client = Self::client(server)?;
        let request = client
            .get(server.endpoint("/api/v1/me"))
            .bearer_auth(token.expose());
        let body: UserBody = Self::body(Self::send(request).await?).await?;
        Ok(CloudAccount {
            id: body.id,
            display_name: body.display_name,
        })
    }

    /// Revokes this token on the server.
    async fn logout(&self, server: &CloudServer, token: &SessionToken) -> Result<(), CallError> {
        let client = Self::client(server)?;
        let request = client
            .post(server.endpoint("/auth/desktop/logout"))
            .bearer_auth(token.expose());
        Self::send(request).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{CallError, CloudApi, HttpCloudApi};
    use crate::cloud_connection::secret::SessionToken;
    use crate::cloud_connection::server::CloudServer;
    use pretty_assertions::assert_eq;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves one fixed answer to every request on a loopback port.
    async fn server_answering(status: &'static str) -> CloudServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let origin = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("addr").port()
        );
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer).await;
                let reply = format!(
                    "HTTP/1.1 {status}\r\nLocation: http://elsewhere.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        CloudServer::parse(&origin).expect("origin")
    }

    /// Only 401 may end a session; server faults and redirects never do.
    #[tokio::test]
    async fn only_unauthorized_rejects_the_token() {
        let token = SessionToken::new("ora_dt_x".to_owned());
        let mut outcomes = Vec::new();
        for status in [
            "401 Unauthorized",
            "503 Service Unavailable",
            "500 Internal Server Error",
            "404 Not Found",
            "302 Found",
        ] {
            let server = server_answering(status).await;
            outcomes.push(match HttpCloudApi.me(&server, &token).await {
                Err(CallError::Rejected) => "rejected",
                Err(CallError::Unreachable(_)) => "unreachable",
                Err(CallError::Request(_)) => "request",
                Ok(_) => "ok",
            });
        }
        assert_eq!(
            outcomes,
            vec![
                "rejected",
                "unreachable",
                "unreachable",
                "request",
                "unreachable"
            ]
        );
        let closed = CloudServer::parse("http://127.0.0.1:9").expect("origin");
        assert!(matches!(
            HttpCloudApi.me(&closed, &token).await,
            Err(CallError::Unreachable(_))
        ));
    }
}
