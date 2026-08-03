use crate::assembler::AssembledRecord;
use crate::clock::{HistoryClock, format_timestamp};
use crate::error::HistoryError;
use crate::path::history_path;
use crate::record::{HistoryLine, HistoryRecord};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Appends settled records to one session's history file.
///
/// Exactly one writer may exist per file. Ora guarantees this structurally: the
/// session's runtime actor owns it, and that actor serializes every operation on
/// the session, so a load can never run while a prompt is appending.
///
/// The file handle is opened per append rather than held open. An append is a
/// handful of syscalls, and on Windows a held handle would block deleting the
/// session while its actor is still winding down.
pub struct HistoryWriter<C: HistoryClock> {
    path: PathBuf,
    clock: C,
}

impl<C: HistoryClock> HistoryWriter<C> {
    /// Resolves where one session's history lives without touching the filesystem.
    ///
    /// Nothing is created until there is something to write, so a session opened
    /// and never prompted leaves no empty directories behind.
    pub fn open(root: &Path, session_id: &str, clock: C) -> Result<Self, HistoryError> {
        Ok(Self {
            path: history_path(root, session_id)?,
            clock,
        })
    }

    /// Returns the file this writer appends to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one record at a caller-chosen position.
    pub fn append_record(&self, seq: u32, record: HistoryRecord) -> Result<(), HistoryError> {
        self.append(&[AssembledRecord { seq, record }])
    }

    /// Appends a batch of records in one open-write-flush cycle.
    ///
    /// The batch is encoded fully before the file is touched, so a record that
    /// cannot be serialized fails without leaving a partial line behind. Writes
    /// are flushed but not synced: losing the last few records to a power cut is
    /// an acceptable trade for keeping a long turn's appends off the disk's
    /// latency path.
    pub fn append(&self, records: &[AssembledRecord]) -> Result<(), HistoryError> {
        if records.is_empty() {
            return Ok(());
        }
        let at = format_timestamp(self.clock.now_local());
        let mut buffer = Vec::new();
        for record in records {
            let line = HistoryLine::new(at.clone(), record.seq, record.record.clone());
            serde_json::to_writer(&mut buffer, &line).map_err(HistoryError::Encode)?;
            buffer.push(b'\n');
        }
        let mut file = self.open_for_append()?;
        file.write_all(&buffer)
            .map_err(|source| HistoryError::Append {
                path: self.path.clone(),
                source,
            })?;
        file.flush().map_err(|source| HistoryError::Append {
            path: self.path.clone(),
            source,
        })
    }

    /// Opens the file, creating its shard directories only if they are missing.
    ///
    /// Trying first and recovering costs one syscall in the common case, where
    /// every append after the first finds the directory already there.
    fn open_for_append(&self) -> Result<File, HistoryError> {
        let mut options = OpenOptions::new();
        let options = options.create(true).append(true);
        match options.open(&self.path) {
            Ok(file) => Ok(file),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                let parent = self.path.parent().unwrap_or(Path::new("."));
                std::fs::create_dir_all(parent).map_err(|source| {
                    HistoryError::DirectoryCreate {
                        path: parent.to_path_buf(),
                        source,
                    }
                })?;
                options
                    .open(&self.path)
                    .map_err(|source| HistoryError::Open {
                        path: self.path.clone(),
                        source,
                    })
            }
            Err(source) => Err(HistoryError::Open {
                path: self.path.clone(),
                source,
            }),
        }
    }
}

/// Removes one session's history file, reporting success even when none existed.
///
/// Ora's soft delete is what a user experiences as deletion, so the history it
/// covers goes with it. A missing file is treated as already removed because the
/// caller's goal is the absence of the file, not the act of deleting it.
pub fn remove_session_history(root: &Path, session_id: &str) -> Result<(), HistoryError> {
    let path = history_path(root, session_id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(HistoryError::Remove { path, source }),
    }
}
