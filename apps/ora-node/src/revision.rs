//! Revision delivery: pushes an ended session's final commit to the Git Server and uploads its
//! history to Cloud's object store, and restores a prior Revision from the Git Server into the
//! checkout of a session that resumes it.
//!
//! A delivery prepares once — an internal snapshot commit under the Revision ref and a copy of the
//! sealed session JSONL — and freezes the final commit, the history's bytes, size and digest, and
//! the declaration it will report, before the first push. A restart after that point pushes the
//! same commit and uploads the same bytes; only upload grants are requested again, since they live
//! in memory only.
//!
//! The worker owns the ledger and decides what runs next; the delivery executor runs Git, file
//! copies and HTTP away from it (see `service::revisions`).

mod grants;
mod prepare;
mod push;
mod restore;
mod snapshot;
mod upload;

pub(crate) use grants::GrantStore;
pub(crate) use prepare::{Preparation, prepare};
pub(crate) use push::{PushJob, Pushed, push};
pub(crate) use restore::RevisionRestorer;
pub(crate) use snapshot::{DeliveryGit, GitPolicy};
pub(crate) use upload::{HttpUploader, ObjectUploader, RetryPolicy, UploadEnd, UploadJob, upload};

/// File name of the history inside a delivery's frozen directory.
const HISTORY_FILE: &str = "history.jsonl";

/// Directory under the Node home holding one frozen directory per running delivery.
pub(crate) const DELIVERY_ROOT: &str = "revision-deliveries";

#[cfg(test)]
mod tests;
