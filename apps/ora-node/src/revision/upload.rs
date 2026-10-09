//! Uploading the frozen objects with memory-only grants (Revision ADR D4, protocol D5).
//!
//! A 412 answer counts as uploaded: a previous PUT may have created the object while its reply was
//! lost, and the conditional create refuses to overwrite it. The Node never infers that the stored
//! bytes match; it reports its frozen declaration and Cloud verifies size and digest itself.
use super::grants::{GrantStore, Registration};
use super::prepare::stored;
use super::{BUNDLE_FILE, HISTORY_FILE};
use ora_node_db::FrozenOutcome;
use ora_node_protocol::*;
use ora_utils::http::{FileUpload, ReqwestUploader, UploadError, UploadOptions};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Sends one object with one grant.
///
/// Implementations perform exactly one request and report its status; retry, grant renewal and
/// what a status means belong to [`upload`]. They must never log or retain the grant.
pub(crate) trait ObjectUploader: Send + Sync + 'static {
    /// Returns the HTTP status of a single upload request.
    fn put(
        &self,
        grant: &ObjectUploadGrant,
        file: &Path,
    ) -> impl Future<Output = Result<u16, UploadError>> + Send;
}

/// The production uploader streams the frozen file with the grant's headers unchanged.
pub(crate) struct HttpUploader {
    client: ReqwestUploader,
    options: UploadOptions,
}

impl HttpUploader {
    pub(crate) fn new(client: ReqwestUploader, options: UploadOptions) -> Self {
        Self { client, options }
    }
}

impl ObjectUploader for HttpUploader {
    /// Uses the method the URL was signed for.
    async fn put(&self, grant: &ObjectUploadGrant, file: &Path) -> Result<u16, UploadError> {
        let method = match grant.method {
            UploadMethod::Put => "PUT",
        };
        self.client
            .send(
                FileUpload {
                    method,
                    url: grant.url.as_str(),
                    headers: &grant.headers,
                    file,
                },
                self.options,
            )
            .await
    }
}

/// Timing of grant requests and retries; tests shorten it, production uses [`RetryPolicy::DEFAULT`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct RetryPolicy {
    /// Failed PUTs per object before the delivery fails with `upload_failed`.
    pub(crate) attempts: u32,
    /// How often an unanswered grant request is repeated while waiting.
    pub(crate) resend: Duration,
    /// Pause before retrying a failed PUT.
    pub(crate) backoff: Duration,
    /// A grant expiring sooner than this is treated as already expired.
    pub(crate) expiry_margin: Duration,
}

impl RetryPolicy {
    pub(crate) const DEFAULT: Self = Self {
        attempts: 3,
        resend: Duration::from_secs(/*secs*/ 2),
        backoff: Duration::from_millis(/*millis*/ 500),
        expiry_margin: Duration::from_secs(/*secs*/ 5),
    };
}

/// One frozen delivery to upload.
#[derive(Clone, Debug)]
pub(crate) struct UploadJob {
    pub(crate) operation: OperationId,
    pub(crate) execution: ExecutionId,
    pub(crate) node_id: NodeId,
    pub(crate) directory: PathBuf,
    pub(crate) outcome: FrozenOutcome,
}

/// How an upload ended; an interrupted upload is never reported here, it is simply resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UploadEnd {
    Uploaded,
    Failed,
}

/// The frozen objects of a plan and the files holding them, bundle first.
pub(crate) fn objects(outcome: &FrozenOutcome) -> Vec<(&StoredObject, &'static str)> {
    match outcome {
        FrozenOutcome::Delivered(result) => {
            vec![
                (&result.bundle, BUNDLE_FILE),
                (&result.history, HISTORY_FILE),
            ]
        }
        FrozenOutcome::Unchanged(result) => vec![(&result.history, HISTORY_FILE)],
    }
}

/// Uploads every frozen object, asking for grants whenever none is valid, until all are stored or
/// one exhausted its attempts. The caller cancels the future to stop at shutdown.
pub(crate) async fn upload<U: ObjectUploader>(
    uploader: &U,
    grants: &GrantStore,
    job: &UploadJob,
    policy: RetryPolicy,
) -> UploadEnd {
    let objects = objects(&job.outcome);
    // The bytes must still be the ones declared; uploading anything else under the same keys
    // would break the declaration Cloud verifies.
    for (object, file) in &objects {
        match stored(&object.key, &job.directory.join(file)) {
            Ok(current) if current == **object => {}
            Ok(_) | Err(_) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), object_key = %object.key.as_str(), "frozen delivery object is missing or changed");
                return UploadEnd::Failed;
            }
        }
    }
    let registration = grants.register(
        job.operation.clone(),
        job.execution.clone(),
        job.node_id.clone(),
        objects
            .iter()
            .map(|(object, _)| (object.key.clone(), object.sha256.clone()))
            .collect(),
    );
    for (object, file) in objects {
        let path = job.directory.join(file);
        if !upload_object(
            uploader,
            grants,
            &registration,
            job,
            &object.key,
            &path,
            policy,
        )
        .await
        {
            return UploadEnd::Failed;
        }
        registration.uploaded(&object.key);
    }
    UploadEnd::Uploaded
}

/// Uploads one object; returns `false` once its attempts are exhausted.
async fn upload_object<U: ObjectUploader>(
    uploader: &U,
    grants: &GrantStore,
    registration: &Registration,
    job: &UploadJob,
    key: &ObjectKey,
    path: &Path,
    policy: RetryPolicy,
) -> bool {
    let mut failures = 0;
    while failures < policy.attempts {
        let grant = wait_for_grant(grants, registration, key, policy).await;
        let status = uploader.put(&grant, path).await;
        match status {
            Ok(200..=299 | 412) => return true,
            Ok(403) => {
                // An expired or revoked signature: only a new grant can succeed.
                registration.discard(key);
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), object_key = %key.as_str(), "object store refused the upload grant");
            }
            Ok(status) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), object_key = %key.as_str(), status, "object upload failed");
                tokio::time::sleep(policy.backoff).await;
            }
            Err(error) => {
                ora_logging::ora_warn!(execution_id = %job.execution.as_str(), object_key = %key.as_str(), error = %error, "object upload failed");
                tokio::time::sleep(policy.backoff).await;
            }
        }
        failures += 1;
    }
    false
}

/// Waits for a usable grant, repeating the request while none arrives. Waiting has no deadline:
/// the Controller may be disconnected for a long time, and the delivery stays running meanwhile.
async fn wait_for_grant(
    grants: &GrantStore,
    registration: &Registration,
    key: &ObjectKey,
    policy: RetryPolicy,
) -> ObjectUploadGrant {
    let mut offers = grants.offers();
    let mut requested: Option<tokio::time::Instant> = None;
    loop {
        offers.borrow_and_update();
        if let Some(grant) = registration.valid(key, policy.expiry_margin) {
            return grant;
        }
        // Grants for other keys also wake this loop; the request itself stays rate-limited.
        if requested.is_none_or(|at| at.elapsed() >= policy.resend) {
            registration.request();
            requested = Some(tokio::time::Instant::now());
        }
        // A closed channel cannot happen while the store is alive; the sleep still bounds the wait.
        let _ = tokio::time::timeout(policy.resend, offers.changed()).await;
    }
}
