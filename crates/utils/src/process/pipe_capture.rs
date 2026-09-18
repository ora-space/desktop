use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// EOF is independent of whether the retained prefix includes every byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeReadState {
    Open,
    Eof,
    Failed(String),
}

/// A bounded copy from an immutable prefix; reading never consumes retained data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeRead {
    pub bytes: Vec<u8>,
    pub retained: usize,
    pub truncated: bool,
    pub state: PipeReadState,
}

struct Buffer {
    bytes: Vec<u8>,
    truncated: bool,
    state: PipeReadState,
}

/// Owns an exclusively supplied pipe and drains it without an attached consumer.
///
/// Only the first `limit` bytes are retained. Overflow is sticky and does not stop draining.
/// Dropping cancels the reader even if another process still holds the write end open.
pub struct PipeCapture {
    buffer: Arc<Mutex<Buffer>>,
    failed: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

impl PipeCapture {
    /// The file must be a pipe with exclusive read ownership; O_NONBLOCK affects its open description.
    pub fn start(mut pipe: File, limit: usize) -> io::Result<Self> {
        // SAFETY: fcntl borrows the live descriptor and preserves its existing status flags.
        let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(io::Error::last_os_error());
        }
        let buffer = Arc::new(Mutex::new(Buffer {
            bytes: Vec::new(),
            truncated: false,
            state: PipeReadState::Open,
        }));
        let failed = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&buffer);
        let failure = Arc::clone(&failed);
        let cancellation = Arc::clone(&cancel);
        let worker = std::thread::Builder::new()
            .name("pipe-capture".into())
            .spawn(move || {
                let mut chunk = [0; 8192];
                while !cancellation.load(Ordering::Acquire) {
                    match pipe.read(&mut chunk) {
                        Ok(size) => {
                            let Ok(mut buffer) = shared.lock() else {
                                failure.store(true, Ordering::Release);
                                return;
                            };
                            if size == 0 {
                                buffer.state = PipeReadState::Eof;
                                return;
                            }
                            let keep = size.min(limit - buffer.bytes.len());
                            if let Err(error) = buffer.bytes.try_reserve_exact(keep) {
                                buffer.state = PipeReadState::Failed(error.to_string());
                                failure.store(true, Ordering::Release);
                                return;
                            }
                            buffer.bytes.extend_from_slice(&chunk[..keep]);
                            if keep < size {
                                buffer.truncated = true;
                                failure.store(true, Ordering::Release);
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            // Cancellation unparks this finite wait; no blocking pipe read can leak a worker.
                            std::thread::park_timeout(Duration::from_millis(/*millis*/ 5));
                        }
                        Err(error) => {
                            if let Ok(mut buffer) = shared.lock() {
                                buffer.state = PipeReadState::Failed(error.to_string());
                            }
                            failure.store(true, Ordering::Release);
                            return;
                        }
                    }
                }
            })?;
        Ok(Self {
            buffer,
            failed,
            cancel,
            worker,
        })
    }

    /// Control paths can observe overflow/read failure without waiting for output copies or I/O.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Copies at most `max_bytes`; offsets beyond the currently retained prefix are rejected.
    pub fn read(&self, offset: usize, max_bytes: usize) -> io::Result<PipeRead> {
        let buffer = self
            .buffer
            .lock()
            .map_err(|_| io::Error::other("pipe reader panicked"))?;
        if offset > buffer.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "offset beyond retained output",
            ));
        }
        let end = offset.saturating_add(max_bytes).min(buffer.bytes.len());
        Ok(PipeRead {
            bytes: buffer.bytes[offset..end].to_vec(),
            retained: buffer.bytes.len(),
            truncated: buffer.truncated,
            state: buffer.state.clone(),
        })
    }
}

impl Drop for PipeCapture {
    /// Never waits for a writer to exit; the worker releases its pipe after cancellation.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.worker.thread().unpark();
    }
}
