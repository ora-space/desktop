use crate::BackendError;

/// Runs one blocking repository operation off the async runtime's worker threads.
///
/// The SQLite work behind a delete genuinely blocks: acquiring a pooled
/// connection waits when every slot is taken, and a cascading delete opens an
/// immediate transaction that parks on the busy timeout while another writer
/// holds the reservation. Parking an async worker for that long starves every
/// other request the runtime is serving, so the wait belongs on the blocking
/// pool even though the caller is asynchronous for unrelated reasons.
pub(crate) async fn spawn_repository_work<T>(
    work: impl FnOnce() -> Result<T, BackendError> + Send + 'static,
) -> Result<T, BackendError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| BackendError::internal("repository operation did not complete", source))?
}
