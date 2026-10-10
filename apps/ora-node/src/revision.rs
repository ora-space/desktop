//! Revision delivery: saves an ended session's checkout and history to Cloud's object store, and
//! restores a prior Revision into the checkout of a session that resumes it.
//!
//! A delivery prepares once — an internal snapshot commit under the Revision ref, a bundle
//! relative to the clone's base commit, and a copy of the sealed session JSONL — and freezes those
//! bytes, their sizes and digests, and the declaration it will report, before the first upload.
//! A restart after that point resumes the same bytes and declaration; only grants are requested
//! again, since they live in memory only.
//!
//! The worker owns the ledger and decides what runs next; the delivery executor runs Git, file
//! copies and HTTP away from it (see `service::revisions`).

mod grants;
mod prepare;
mod restore;
mod snapshot;
mod upload;

pub(crate) use grants::GrantStore;
pub(crate) use prepare::{Preparation, prepare};
pub(crate) use restore::{
    DownloadGrants, HttpDownloader, RESTORE_ROOT, RestorePolicy, RevisionRestorer,
    purge as purge_restores,
};
pub(crate) use snapshot::{DeliveryGit, GitPolicy};
pub(crate) use upload::{HttpUploader, ObjectUploader, RetryPolicy, UploadEnd, UploadJob, upload};

/// File names inside a delivery's frozen directory.
const BUNDLE_FILE: &str = "revision.bundle";
const HISTORY_FILE: &str = "history.jsonl";

/// Directory under the Node home holding one frozen directory per running delivery.
pub(crate) const DELIVERY_ROOT: &str = "revision-deliveries";

#[cfg(test)]
mod tests;
