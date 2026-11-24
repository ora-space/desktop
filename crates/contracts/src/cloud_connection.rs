//! Desktop's connection to one Cloud server: the server, the signed-in account and its state.
//!
//! The session token never appears here. The host keeps it in the operating system's secure
//! store, so these DTOs describe only what the main webview may show.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The Cloud account the stored token belongs to, as the server reported it at sign-in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct CloudAccount {
    pub id: String,
    pub display_name: String,
}

/// Why a sign-in ended without a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub enum CloudLoginFailure {
    /// The person cancelled on the server's approval page (it redirects with
    /// `error=access_denied`), or the page returned another error instead of a code.
    Denied,
    /// Nobody approved in the browser within the time allowed.
    TimedOut,
    /// The server refused to exchange the code, for example because it expired.
    Rejected,
    /// The server could not be reached, or failed, during the exchange.
    Unreachable,
}

/// The sign-in state for the configured server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub enum CloudSession {
    /// No token is stored for this server. `failure` explains why the last sign-in ended without
    /// one; it is absent after a sign-out, a cancelled sign-in or a restart.
    SignedOut { failure: Option<CloudLoginFailure> },
    /// The system browser is open on the server's approval page and the host is waiting for it.
    SigningIn,
    /// The token was accepted by the server's most recent answer.
    SignedIn { account: CloudAccount },
    /// A token is stored but the server could not be reached or failed; it is kept for later.
    Unreachable { account: CloudAccount },
    /// The server rejected the token (revoked, expired or account disabled); it was deleted.
    NeedsSignIn,
    /// The operating system's secure store could not be used, so Cloud features are off.
    StorageUnavailable { reason: String },
}

/// Everything the main webview may know about the Cloud connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub enum CloudConnection {
    /// No server address is saved; Desktop works locally only.
    NotConfigured,
    /// A server origin is saved, normalized as `scheme://host[:port]`.
    Configured {
        server: String,
        session: CloudSession,
    },
}

/// Requests the current connection without additional parameters.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct GetCloudConnectionRequest {}

/// Replaces the saved server address; `null` forgets it.
///
/// Changing or forgetting the server cancels a sign-in in progress and signs out of the previous
/// server first. An address that is not a usable origin is rejected as an invalid request.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct SetCloudServerRequest {
    pub server: Option<String>,
}

/// Opens the system browser on the configured server's approval page.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct StartCloudLoginRequest {}

/// Abandons the sign-in in progress, if any.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct CancelCloudLoginRequest {}

/// Signs this Desktop out of the configured server.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct LogoutCloudRequest {}

/// Whether the server confirmed that the token it issued is revoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub enum CloudRevocation {
    /// The server answered that the token is gone.
    Confirmed,
    /// The server could not be reached or failed; only this device forgot the token.
    Unconfirmed,
}

/// The connection after a sign-out, and whether the server confirmed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct LogoutCloudResponse {
    pub connection: CloudConnection,
    pub revocation: CloudRevocation,
}

/// The connection after saving or forgetting the server address.
///
/// `revocation` is present only when the previous server had a token: it was deleted on this
/// device and the previous server was asked to revoke it, which it may not have confirmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cloud-connection.ts")]
pub struct SetCloudServerResponse {
    pub connection: CloudConnection,
    pub revocation: Option<CloudRevocation>,
}

/// Exports the Cloud connection DTO family.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    CloudAccount::export(config)?;
    CloudLoginFailure::export(config)?;
    CloudSession::export(config)?;
    CloudConnection::export(config)?;
    GetCloudConnectionRequest::export(config)?;
    SetCloudServerRequest::export(config)?;
    StartCloudLoginRequest::export(config)?;
    CancelCloudLoginRequest::export(config)?;
    LogoutCloudRequest::export(config)?;
    CloudRevocation::export(config)?;
    LogoutCloudResponse::export(config)?;
    SetCloudServerResponse::export(config)?;
    Ok(())
}
