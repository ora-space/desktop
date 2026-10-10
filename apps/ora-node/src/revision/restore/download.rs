//! Downloading a prior bundle with a memory-only grant (Node restore ADR D2).
//!
//! The bytes land in a Node-private file and count only once their size and SHA-256 match the
//! object Cloud fixed in the session input. Network failures and 5xx answers are retried a few
//! times; a 403 means the grant expired or was revoked, so one fresh grant is requested; a
//! refusal or any other answer fails the restore at once. The caller bounds the whole wait.
use super::RestorePolicy;
use super::grants::{Answer, DownloadGrants, DownloadRegistration};
use ora_node_protocol::{ExecutionId, NodeId, ObjectDownloadGrant, OperationId, StoredObject};
use ora_utils::http::{FetchError, FetchOptions, FetchOutcome, FileFetch, ReqwestFetcher};
use std::future::Future;
use std::path::{Path, PathBuf};

/// Reads one object with one grant.
///
/// Implementations perform exactly one request, never follow a redirect, write a success body to
/// a new file at `destination` and measure it; retry, grant renewal and verification belong to
/// [`download`]. They must never log or retain the grant.
pub(crate) trait ObjectDownloader: Send + Sync + 'static {
    /// Returns the stored body's measurements, or the status of any other answer.
    fn get(
        &self,
        grant: &ObjectDownloadGrant,
        destination: &Path,
        limit: u64,
    ) -> impl Future<Output = Result<FetchOutcome, FetchError>> + Send;
}

/// The production downloader sends the grant's headers unchanged with a single `GET`.
pub(crate) struct HttpDownloader {
    fetcher: ReqwestFetcher,
    options: FetchOptions,
}

impl HttpDownloader {
    pub(crate) fn new(fetcher: ReqwestFetcher, options: FetchOptions) -> Self {
        Self { fetcher, options }
    }
}

impl ObjectDownloader for HttpDownloader {
    /// The method is always `GET`: the protocol has no other download method.
    async fn get(
        &self,
        grant: &ObjectDownloadGrant,
        destination: &Path,
        limit: u64,
    ) -> Result<FetchOutcome, FetchError> {
        self.fetcher
            .fetch(
                FileFetch {
                    url: grant.url.as_str(),
                    headers: &grant.headers,
                    destination,
                    max_bytes: limit,
                },
                self.options,
            )
            .await
    }
}

/// One prior bundle to download.
#[derive(Clone, Debug)]
pub(crate) struct DownloadJob {
    pub(crate) operation: OperationId,
    pub(crate) execution: ExecutionId,
    pub(crate) node_id: NodeId,
    pub(crate) bundle: StoredObject,
    pub(crate) destination: PathBuf,
}

/// Why the bundle could not be downloaded; every case fails the restore as unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DownloadFailure {
    #[error("Cloud refused the download grant")]
    Refused,
    #[error("the object store refused the download")]
    Rejected,
    #[error("the downloaded bundle does not match its declared size and digest")]
    Mismatch,
    #[error("the download kept failing")]
    Exhausted,
}

/// Downloads and verifies the bundle, asking for grants whenever none is valid. The caller
/// cancels the future to stop, which drops the registration and any grant with it.
pub(crate) async fn download<D: ObjectDownloader>(
    downloader: &D,
    grants: &DownloadGrants,
    job: &DownloadJob,
    policy: RestorePolicy,
) -> Result<(), DownloadFailure> {
    let registration = grants.register(
        job.operation.clone(),
        job.execution.clone(),
        job.node_id.clone(),
        job.bundle.key.clone(),
    );
    let mut failures = 0;
    let mut renewed = false;
    loop {
        let grant = wait_for_grant(grants, &registration, policy).await?;
        // A failed attempt never leaves bytes behind, but a cancelled one may have.
        let _ = std::fs::remove_file(&job.destination);
        let fetched = downloader
            .get(&grant, &job.destination, job.bundle.size)
            .await;
        let retry = match fetched {
            Ok(FetchOutcome::Stored { bytes, sha256 }) => {
                let digest: String = sha256.iter().map(|byte| format!("{byte:02x}")).collect();
                if bytes == job.bundle.size && digest == job.bundle.sha256.as_str() {
                    return Ok(());
                }
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), "prior bundle does not match its declared size and digest");
                return Err(DownloadFailure::Mismatch);
            }
            // An expired or revoked signature: only a new grant can succeed, and only once.
            Ok(FetchOutcome::Status(403)) if !renewed => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), "object store refused the download grant; asking for a fresh one");
                registration.discard();
                renewed = true;
                continue;
            }
            Ok(FetchOutcome::Status(status @ (429 | 500..=599))) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), status, "prior bundle download failed");
                true
            }
            Ok(FetchOutcome::Status(status)) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), status, "object store refused the prior bundle");
                return Err(DownloadFailure::Rejected);
            }
            Err(error @ (FetchError::Network(_) | FetchError::Timeout)) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), error = %error, "prior bundle download failed");
                true
            }
            Err(FetchError::TooLarge { .. }) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), "prior bundle is larger than declared");
                return Err(DownloadFailure::Mismatch);
            }
            Err(error @ (FetchError::InvalidRequest(_) | FetchError::Io { .. })) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), error = %error, "prior bundle could not be downloaded");
                return Err(DownloadFailure::Rejected);
            }
        };
        failures += u32::from(retry);
        if failures > policy.retries {
            return Err(DownloadFailure::Exhausted);
        }
        tokio::time::sleep(policy.backoff).await;
    }
}

/// Waits for an answer, repeating the request while none arrives; the caller's deadline bounds
/// the wait.
async fn wait_for_grant(
    grants: &DownloadGrants,
    registration: &DownloadRegistration,
    policy: RestorePolicy,
) -> Result<ObjectDownloadGrant, DownloadFailure> {
    let mut offers = grants.offers();
    let mut requested: Option<tokio::time::Instant> = None;
    loop {
        offers.borrow_and_update();
        match registration.answer(policy.expiry_margin) {
            Some(Answer::Granted(grant)) => return Ok(grant),
            Some(Answer::Refused) => return Err(DownloadFailure::Refused),
            None => {}
        }
        if requested.is_none_or(|at| at.elapsed() >= policy.resend) {
            registration.request();
            requested = Some(tokio::time::Instant::now());
        }
        // A closed channel cannot happen while the store is alive; the sleep still bounds the wait.
        let _ = tokio::time::timeout(policy.resend, offers.changed()).await;
    }
}
