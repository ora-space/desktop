//! SQLite adapter of [`CoordinationStore`]: the local durable store behind clone coordination. It
//! owns the database file, the OS lease beside it and every transaction; nothing else in the crate
//! issues SQL, and cloud deployments never open it.
mod operations;
mod takeover;

use crate::*;
use ora_utils::fs::{ExclusiveFileLock, ExclusiveLockError};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::{self, OpenOptions},
    future::Future,
    io,
    path::Path,
    sync::{Arc, Mutex},
};

const APPLICATION_ID: i64 = 0x4f524143;
const SCHEMA: &str = include_str!("schema.sql");

/// The database lease and transaction owner retain original dispatches, results and event receipts.
/// Clones share one connection; every operation runs on the blocking pool so SQLite's fsync never
/// stalls the async runtime that hosts Node sessions and the API.
pub struct SqliteStore<W = DurableWrites> {
    inner: Arc<Mutex<Inner<W>>>,
    id: ControllerId,
}

impl<W> Clone for SqliteStore<W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            id: self.id.clone(),
        }
    }
}

/// Connection, fault seam and lease live together so the last handle releases them in order.
struct Inner<W> {
    connection: Connection,
    writes: W,
    // Held beside the database rather than on it so SQLite's own locks never collide with ours.
    _lease: ExclusiveFileLock,
}

impl SqliteStore {
    /// Opens explicitly injected local state, preserving unknown files instead of reinitializing them.
    pub fn open(home: &Path, id: ControllerId) -> Result<Self, Error> {
        Self::open_with_guard(home, id, DurableWrites)
    }
}

impl<W: WriteGuard> SqliteStore<W> {
    /// Injects persistence failure boundaries without substituting the durable store or OS lease.
    pub fn open_with_guard(home: &Path, id: ControllerId, writes: W) -> Result<Self, Error> {
        if !home.is_absolute() || id.as_str().trim().is_empty() {
            return Err(Error::InvalidStorage);
        }
        let mut directory = fs::DirBuilder::new();
        directory.recursive(/*recursive*/ true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(/*mode*/ 0o700);
        }
        directory.create(home)?;
        #[cfg(target_os = "linux")]
        {
            // SAFETY: reads the local identity without changing credentials.
            let uid = unsafe { libc::geteuid() };
            ora_utils::path::open_private_path(
                home,
                uid,
                ora_utils::path::TrustedPathKind::Directory,
            )?;
        }
        let path = home.join("ora-controller.sqlite3");
        // Creating the file first settles `created` atomically and rejects non-files before any
        // lease is taken beside them.
        let created = match OpenOptions::new()
            .read(/*read*/ true)
            .write(/*write*/ true)
            .create_new(/*create_new*/ true)
            .open(&path)
        {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !fs::symlink_metadata(&path)?.is_file() {
                    return Err(Error::InvalidStorage);
                }
                drop(
                    OpenOptions::new()
                        .read(/*read*/ true)
                        .write(/*write*/ true)
                        .open(&path)?,
                );
                false
            }
            Err(error) => return Err(error.into()),
        };
        let lease = ExclusiveFileLock::try_acquire(&home.join("ora-controller.sqlite3.lock"))
            .map_err(|error| match error {
                ExclusiveLockError::Busy { .. } => Error::AlreadyRunning,
                ExclusiveLockError::Io { source, .. } => Error::Io(source),
            })?;
        let mut connection = Connection::open(path)?;
        if created {
            let tx = connection.transaction()?;
            tx.execute_batch(SCHEMA)?;
            tx.execute(
                "INSERT INTO controller_metadata VALUES (1,?1)",
                [id.as_str()],
            )?;
            tx.pragma_update(/*schema_name*/ None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(
                /*schema_name*/ None,
                "user_version",
                /*pragma_value*/ 1,
            )?;
            tx.commit()?;
        }
        let app: i64 = connection.pragma_query_value(
            /*schema_name*/ None,
            "application_id",
            |r| r.get(/*idx*/ 0),
        )?;
        let version: i64 = connection.pragma_query_value(
            /*schema_name*/ None,
            "user_version",
            |r| r.get(/*idx*/ 0),
        )?;
        if app != APPLICATION_ID || version != 1 {
            return Err(Error::InvalidStorage);
        }
        let expected = Connection::open_in_memory()?;
        expected.execute_batch(SCHEMA)?;
        let schema =
            |connection: &Connection| -> Result<Vec<(String, String, String)>, rusqlite::Error> {
                connection.prepare("SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?
                .query_map([], |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?, r.get(/*idx*/ 2)?)))?.collect()
            };
        let integrity: String = connection.pragma_query_value(
            /*schema_name*/ None,
            "integrity_check",
            |r| r.get(/*idx*/ 0),
        )?;
        let foreign: i64 =
            connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(/*idx*/ 0)
            })?;
        if integrity != "ok" || foreign != 0 || schema(&connection)? != schema(&expected)? {
            return Err(Error::InvalidStorage);
        }
        let stored: String = connection.query_row(
            "SELECT controller FROM controller_metadata WHERE singleton=1",
            [],
            |r| r.get(/*idx*/ 0),
        )?;
        if stored != id.as_str() {
            return Err(Error::InvalidStorage);
        }
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "ON")?;
        connection.pragma_update(/*schema_name*/ None, "synchronous", "FULL")?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                connection,
                writes,
                _lease: lease,
            })),
            id,
        })
    }

    /// Runs one short transaction on the blocking pool; the mutex serializes the single connection.
    async fn blocking<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut Inner<W>) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut inner = inner
                .lock()
                .map_err(|_| io::Error::other("SQLite store lock poisoned"))?;
            action(&mut inner)
        })
        .await
        .map_err(|error| io::Error::other(error.to_string()))?
    }
}

impl<W: WriteGuard> CoordinationStore for SqliteStore<W> {
    fn id(&self) -> &ControllerId {
        &self.id
    }

    fn take_over_node_event(
        &self,
        session: &NodeRuntimeIdentity,
        event: &CloneResultMessage,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        let (session, event) = (session.clone(), event.clone());
        self.blocking(move |inner| inner.take_over_event(&session, &event))
    }

    fn record_queried_result(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        result: &CloneExecutionResult,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        let (session, operation, execution, result) = (
            session.clone(),
            operation.clone(),
            execution.clone(),
            result.clone(),
        );
        self.blocking(move |inner| {
            inner.record_queried_result(&session, &operation, &execution, &result)
        })
    }

    fn original_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> impl Future<Output = Result<CloneRepositoryMessage, Error>> + Send {
        let (session, operation, execution) =
            (session.clone(), operation.clone(), execution.clone());
        self.blocking(move |inner| inner.original(&session, &operation, &execution))
    }

    fn pending_dispatches(
        &self,
        node: &NodeId,
    ) -> impl Future<Output = Result<Vec<CloneRepositoryMessage>, Error>> + Send {
        let node = node.clone();
        self.blocking(move |inner| inner.pending_dispatches(&node))
    }

    fn result(
        &self,
        execution: &ExecutionId,
    ) -> impl Future<Output = Result<Option<ExecutionOutcome>, Error>> + Send {
        let execution = execution.clone();
        self.blocking(move |inner| {
            Ok(inner
                .result(&execution)?
                .as_ref()
                .map(ExecutionOutcome::from))
        })
    }

    /// The local authority is this process; there is no lease to keep and no queue to claim.
    /// The JSON surface records the dispatch when it accepts a request, so there is nothing to gate.
    fn static_node_established(&self, _node: &NodeRuntimeIdentity) {}

    async fn serve(&self, shutdown: impl Future<Output = ()> + Send + 'static) -> io::Result<()> {
        shutdown.await;
        Ok(())
    }
}

impl<W: WriteGuard> CloneIntake for SqliteStore<W> {
    fn accept_request(
        &self,
        request: RequestId,
        spec: CloneExecutionSpec,
    ) -> impl Future<Output = Result<CloneRepositoryMessage, Error>> + Send {
        self.blocking(move |inner| inner.accept(request, spec))
    }

    fn operations(&self) -> impl Future<Output = Result<Vec<CloneOperation>, Error>> + Send {
        self.blocking(|inner| inner.operations())
    }

    fn operation(
        &self,
        execution: &ExecutionId,
    ) -> impl Future<Output = Result<Option<CloneOperation>, Error>> + Send {
        let execution = execution.clone();
        self.blocking(move |inner| inner.operation(&execution))
    }
}

impl<W: WriteGuard> Inner<W> {
    /// Atomically freezes caller intent and original dispatch IDs before any network operation.
    fn accept(
        &mut self,
        request: RequestId,
        spec: CloneExecutionSpec,
    ) -> Result<CloneRepositoryMessage, Error> {
        let command = CloneRepositoryMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(request.clone()),
            operation_id: OperationId::new(uuid::Uuid::new_v4().to_string()),
            execution_id: ExecutionId::new(uuid::Uuid::new_v4().to_string()),
            payload: CloneRepository { spec },
        };
        command.validate()?;
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT input FROM clone_operations WHERE request=?1",
                [request.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        if let Some(input) = existing {
            let original: CloneRepositoryMessage = serde_json::from_str(&input)?;
            return if original.payload == command.payload {
                Ok(original)
            } else {
                Err(Error::Conflict)
            };
        }
        self.writes.before_write(WritePoint::Accept)?;
        self.connection.execute(
            "INSERT INTO clone_operations VALUES (?1,?2,?3,?4,?5,NULL)",
            params![
                request.as_str(),
                command.operation_id.as_str(),
                command.execution_id.as_str(),
                command.payload.spec.node_id.as_str(),
                serde_json::to_string(&command)?
            ],
        )?;
        Ok(command)
    }

    /// Queries the durable original result without acknowledging any Node event.
    fn result(&self, execution: &ExecutionId) -> Result<Option<CloneExecutionResult>, Error> {
        let result: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT result FROM clone_operations WHERE execution=?1",
                [execution.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        result
            .flatten()
            .map(|value| serde_json::from_str(&value).map_err(Error::from))
            .transpose()
    }

    /// Restores the original commands still awaiting a result on one Node, in acceptance order.
    fn pending_dispatches(&self, node: &NodeId) -> Result<Vec<CloneRepositoryMessage>, Error> {
        self.connection
            .prepare(
                "SELECT input FROM clone_operations WHERE node=?1 AND result IS NULL ORDER BY rowid",
            )?
            .query_map([node.as_str()], |r| r.get::<_, String>(/*idx*/ 0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
}
