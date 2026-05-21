use crate::history::OutputHistory;
use crate::process::{PtyProcessFactory, PtyProcessFactoryError, PtyProcessSpawnRequest, PtySize};
use crate::types::{
    PtyClientAttachmentState, PtyCommand, PtyLifecycleEvent, PtySessionControlError, PtySessionId,
    PtySessionReplay, PtySessionStatus,
};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

/// Describes the startup inputs required to create one PTY runtime session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtySessionStartRequest {
    pub session_id: PtySessionId,
    pub cwd: std::path::PathBuf,
    pub program: String,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

/// Exposes one started PTY session to callers that need lifecycle metadata.
#[derive(Debug, Clone)]
pub struct PtySessionHandle {
    pub session_id: PtySessionId,
    pub session_token: CancellationToken,
}

/// Carries the replay snapshot and live-event receiver returned when a client attaches.
pub struct PtySessionAttachment {
    pub state: PtyClientAttachmentState,
    pub replay: PtySessionReplay,
    pub session_token: CancellationToken,
    pub output_receiver: broadcast::Receiver<PtyOutputChunkEvent>,
}

/// Emits live PTY output and exit notifications to one attached client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyOutputChunkEvent {
    Output { data: String },
    Exit { exit_code: Option<i32> },
}

/// Captures PTY runtime failures that occur before or during session start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PtyRuntimeManagerError {
    #[error("pty session already exists: {session_id}")]
    SessionAlreadyExists { session_id: PtySessionId },
    #[error(transparent)]
    Spawn(#[from] PtyProcessFactoryError),
}

/// Owns PTY lifecycle, replay history, and single-client attachment rules for live sessions.
pub struct PtyRuntimeManager<Factory> {
    process_factory: Factory,
    server_token: CancellationToken,
    lifecycle_sender: broadcast::Sender<PtyLifecycleEvent>,
    sessions: Arc<Mutex<HashMap<PtySessionId, Arc<SessionState>>>>,
    history_limit_bytes: usize,
}

impl<Factory> PtyRuntimeManager<Factory>
where
    Factory: PtyProcessFactory + Send + Sync + 'static,
{
    /// Builds a PTY runtime manager rooted in the provided server-owned cancellation token.
    pub fn new(
        process_factory: Factory,
        server_token: CancellationToken,
        history_limit_bytes: usize,
    ) -> Self {
        let (lifecycle_sender, _) = broadcast::channel(32);

        Self {
            process_factory,
            server_token,
            lifecycle_sender,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            history_limit_bytes,
        }
    }

    /// Returns a lifecycle-event receiver so adapters can synchronize persisted session state.
    pub fn subscribe_lifecycle(&self) -> broadcast::Receiver<PtyLifecycleEvent> {
        self.lifecycle_sender.subscribe()
    }

    /// Starts one PTY runtime and begins the background IO and exit watchers that own it.
    pub fn start_session(
        &self,
        request: PtySessionStartRequest,
    ) -> Result<PtySessionHandle, PtyRuntimeManagerError> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sessions.contains_key(&request.session_id) {
            return Err(PtyRuntimeManagerError::SessionAlreadyExists {
                session_id: request.session_id,
            });
        }

        let session_token = self.server_token.child_token();
        let spawned_process = self.process_factory.spawn(PtyProcessSpawnRequest {
            cwd: request.cwd,
            program: request.program,
            args: request.args,
            size: PtySize {
                cols: request.cols,
                rows: request.rows,
            },
        })?;
        let session_id = request.session_id;
        let output_sender = broadcast::channel(128).0;
        let (command_sender, command_receiver) = mpsc::unbounded_channel();
        let state = Arc::new(SessionState {
            session_id: session_id.clone(),
            status: Mutex::new(PtySessionStatus::Running),
            attachment_state: Mutex::new(PtyClientAttachmentState::Detached),
            history: Mutex::new(OutputHistory::new(self.history_limit_bytes)),
            output_sender,
            command_sender: Mutex::new(command_sender),
            session_token: session_token.clone(),
        });

        sessions.insert(session_id.clone(), state.clone());
        drop(sessions);

        spawn_reader_loop(state.clone(), spawned_process.io.reader);
        spawn_writer_loop(
            state.clone(),
            spawned_process.io.writer,
            command_receiver,
            spawned_process.handle.clone(),
        );
        spawn_exit_waiter(
            self.sessions.clone(),
            self.lifecycle_sender.clone(),
            state,
            spawned_process.handle,
        );

        Ok(PtySessionHandle {
            session_id,
            session_token,
        })
    }

    /// Attaches one client to a running PTY session and returns replay plus a live-event stream.
    pub fn attach_session(
        &self,
        session_id: &PtySessionId,
    ) -> Result<PtySessionAttachment, PtySessionControlError> {
        let session = self.session(session_id)?;
        let mut attachment_state = session
            .attachment_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let status = *session
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if status == PtySessionStatus::Exited {
            return Err(PtySessionControlError::SessionExited {
                session_id: session_id.clone(),
            });
        }
        if *attachment_state == PtyClientAttachmentState::Attached {
            return Err(PtySessionControlError::AlreadyAttached {
                session_id: session_id.clone(),
            });
        }

        *attachment_state = PtyClientAttachmentState::Attached;

        let (replay, output_receiver) = {
            let history = session
                .history
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            (history.replay(), session.output_sender.subscribe())
        };

        Ok(PtySessionAttachment {
            state: PtyClientAttachmentState::Attached,
            replay,
            session_token: session.session_token.clone(),
            output_receiver,
        })
    }

    /// Detaches the current live client while leaving the PTY runtime alive for reattachment.
    pub fn detach_session(&self, session_id: &PtySessionId) -> Result<(), PtySessionControlError> {
        let session = self.session(session_id)?;
        let mut attachment_state = session
            .attachment_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        *attachment_state = PtyClientAttachmentState::Detached;

        Ok(())
    }

    /// Sends raw terminal input into one running PTY session.
    pub fn send_input(
        &self,
        session_id: &PtySessionId,
        data: String,
    ) -> Result<(), PtySessionControlError> {
        self.send_command(session_id, PtyCommand::Input { data })
    }

    /// Applies a resize request to one running PTY session.
    pub fn resize_session(
        &self,
        session_id: &PtySessionId,
        cols: u16,
        rows: u16,
    ) -> Result<(), PtySessionControlError> {
        self.send_command(session_id, PtyCommand::Resize { cols, rows })
    }

    /// Requests explicit PTY termination for one running terminal session.
    pub fn kill_session(&self, session_id: &PtySessionId) -> Result<(), PtySessionControlError> {
        self.send_command(session_id, PtyCommand::Kill)
    }

    /// Reports whether one session currently has a live attached client.
    pub fn attachment_state(
        &self,
        session_id: &PtySessionId,
    ) -> Result<PtyClientAttachmentState, PtySessionControlError> {
        let session = self.session(session_id)?;

        Ok(*session
            .attachment_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// Looks up one session runtime and returns a stable missing-session error when absent.
    fn session(
        &self,
        session_id: &PtySessionId,
    ) -> Result<Arc<SessionState>, PtySessionControlError> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
            .cloned()
            .ok_or_else(|| PtySessionControlError::SessionMissing {
                session_id: session_id.clone(),
            })
    }

    /// Delivers one control command into the running PTY session command loop.
    fn send_command(
        &self,
        session_id: &PtySessionId,
        command: PtyCommand,
    ) -> Result<(), PtySessionControlError> {
        let session = self.session(session_id)?;
        let status = *session
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if status == PtySessionStatus::Exited {
            return Err(PtySessionControlError::SessionExited {
                session_id: session_id.clone(),
            });
        }

        session
            .command_sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(command)
            .map_err(|error| PtySessionControlError::ControlFailed {
                message: error.to_string(),
            })
    }
}

/// Stores the mutable runtime state owned by one live PTY session.
struct SessionState {
    session_id: PtySessionId,
    status: Mutex<PtySessionStatus>,
    attachment_state: Mutex<PtyClientAttachmentState>,
    history: Mutex<OutputHistory>,
    output_sender: broadcast::Sender<PtyOutputChunkEvent>,
    command_sender: Mutex<mpsc::UnboundedSender<PtyCommand>>,
    session_token: CancellationToken,
}
/// Streams PTY output into the bounded history and the live output broadcast channel.
fn spawn_reader_loop(session: Arc<SessionState>, mut reader: Box<dyn Read + Send>) {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];

        loop {
            if session.session_token.is_cancelled() {
                break;
            }

            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(bytes_read) => {
                    let data = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                    session
                        .history
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(data.clone());
                    let _ = session
                        .output_sender
                        .send(PtyOutputChunkEvent::Output { data });
                }
                Err(_) => break,
            }
        }
    });
}

/// Applies runtime commands without tying PTY lifetime to one attached client.
fn spawn_writer_loop(
    session: Arc<SessionState>,
    mut writer: Box<dyn Write + Send>,
    mut command_receiver: mpsc::UnboundedReceiver<PtyCommand>,
    process_handle: Arc<dyn crate::process::PtyProcessHandle>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = session.session_token.cancelled() => {
                    let _ = process_handle.kill();
                    break;
                }
                maybe_command = command_receiver.recv() => {
                    match maybe_command {
                        Some(PtyCommand::Input { data }) => {
                            if writer.write_all(data.as_bytes()).is_err() || writer.flush().is_err() {
                                break;
                            }
                        }
                        Some(PtyCommand::Resize { cols, rows }) => {
                            let _ = process_handle.resize(PtySize { cols, rows });
                        }
                        Some(PtyCommand::Kill) => {
                            let _ = process_handle.kill();
                        }
                        None => break,
                    }
                }
            }
        }
    });
}

/// Waits for PTY exit, emits terminal-final events, detaches clients, and clears runtime state.
fn spawn_exit_waiter(
    sessions: Arc<Mutex<HashMap<PtySessionId, Arc<SessionState>>>>,
    lifecycle_sender: broadcast::Sender<PtyLifecycleEvent>,
    session: Arc<SessionState>,
    process_handle: Arc<dyn crate::process::PtyProcessHandle>,
) {
    std::thread::spawn(move || {
        let exit_code = process_handle.wait().ok().and_then(|exit| exit.exit_code);

        session.session_token.cancel();
        *session
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = PtySessionStatus::Exited;
        *session
            .attachment_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            PtyClientAttachmentState::Detached;
        let _ = session
            .output_sender
            .send(PtyOutputChunkEvent::Exit { exit_code });
        session
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&session.session_id);
        let _ = lifecycle_sender.send(PtyLifecycleEvent::Exited {
            session_id: session.session_id.clone(),
            exit_code,
        });
    });
}

#[cfg(test)]
mod tests {
    use super::{PtyRuntimeManager, PtySessionStartRequest};
    use crate::process::{
        PtyChildExit, PtyIoHandle, PtyProcess, PtyProcessFactory, PtyProcessFactoryError,
        PtyProcessHandle, PtyProcessId, PtyProcessSpawnRequest, PtySize,
    };
    use crate::types::{
        PtyClientAttachmentState, PtyLifecycleEvent, PtyServerToken, PtySessionControlError,
        PtySessionId,
    };
    use pretty_assertions::assert_eq;
    use std::io::{Cursor, Result as IoResult, Write};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Verifies a running session replays buffered output after the client detaches and reattaches.
    #[tokio::test]
    async fn replays_history_after_reattach() {
        let factory = FakePtyProcessFactory::with_output("hello\n");
        let server_token = PtyServerToken::new();
        let manager = PtyRuntimeManager::new(factory, server_token.cancellation_token(), 1024);

        manager
            .start_session(PtySessionStartRequest {
                session_id: PtySessionId::new("session-1"),
                cwd: std::path::PathBuf::from("/tmp/task-1"),
                program: "/bin/bash".to_string(),
                args: Vec::new(),
                cols: 80,
                rows: 24,
            })
            .unwrap_or_else(|error| panic!("expected session startup to succeed: {error}"));
        tokio::time::sleep(Duration::from_millis(30)).await;
        let first_attachment = manager
            .attach_session(&PtySessionId::new("session-1"))
            .unwrap_or_else(|error| panic!("expected first attachment to succeed: {error:?}"));

        assert_eq!(
            first_attachment.replay.chunks,
            vec![crate::types::PtyOutputChunk {
                data: "hello\n".to_string(),
            }]
        );
        manager
            .detach_session(&PtySessionId::new("session-1"))
            .unwrap_or_else(|error| panic!("expected detach to succeed: {error:?}"));

        let second_attachment = manager
            .attach_session(&PtySessionId::new("session-1"))
            .unwrap_or_else(|error| panic!("expected second attachment to succeed: {error:?}"));

        assert_eq!(second_attachment.state, PtyClientAttachmentState::Attached);
        assert_eq!(
            second_attachment.replay.chunks,
            vec![crate::types::PtyOutputChunk {
                data: "hello\n".to_string(),
            }]
        );
    }

    /// Verifies the runtime rejects a second concurrent live attachment for the same session.
    #[tokio::test]
    async fn rejects_duplicate_attachment() {
        let factory = FakePtyProcessFactory::with_output("hello\n");
        let server_token = PtyServerToken::new();
        let manager = PtyRuntimeManager::new(factory, server_token.cancellation_token(), 1024);

        manager
            .start_session(PtySessionStartRequest {
                session_id: PtySessionId::new("session-1"),
                cwd: std::path::PathBuf::from("/tmp/task-1"),
                program: "/bin/bash".to_string(),
                args: Vec::new(),
                cols: 80,
                rows: 24,
            })
            .unwrap_or_else(|error| panic!("expected session startup to succeed: {error}"));
        manager
            .attach_session(&PtySessionId::new("session-1"))
            .unwrap_or_else(|error| panic!("expected first attachment to succeed: {error:?}"));

        assert_eq!(
            manager
                .attach_session(&PtySessionId::new("session-1"))
                .map(|_| ()),
            Err(PtySessionControlError::AlreadyAttached {
                session_id: PtySessionId::new("session-1"),
            })
        );
    }

    /// Verifies input, resize, and shutdown requests flow through the runtime-owned control surfaces.
    #[tokio::test]
    async fn forwards_terminal_control_and_server_shutdown() {
        let factory = FakePtyProcessFactory::with_output("ready\n");
        let server_token = PtyServerToken::new();
        let controls = factory.controls();
        let manager = PtyRuntimeManager::new(factory, server_token.cancellation_token(), 1024);
        let mut lifecycle_receiver = manager.subscribe_lifecycle();

        manager
            .start_session(PtySessionStartRequest {
                session_id: PtySessionId::new("session-1"),
                cwd: std::path::PathBuf::from("/tmp/task-1"),
                program: "/bin/bash".to_string(),
                args: Vec::new(),
                cols: 80,
                rows: 24,
            })
            .unwrap_or_else(|error| panic!("expected session startup to succeed: {error}"));
        manager
            .send_input(&PtySessionId::new("session-1"), "pwd\n".to_string())
            .unwrap_or_else(|error| panic!("expected input to succeed: {error:?}"));
        manager
            .resize_session(&PtySessionId::new("session-1"), 120, 50)
            .unwrap_or_else(|error| panic!("expected resize to succeed: {error:?}"));
        tokio::time::sleep(Duration::from_millis(30)).await;
        server_token.cancel();

        assert_eq!(controls.writes(), vec!["pwd\n".to_string()]);
        assert_eq!(
            controls.resizes(),
            vec![PtySize {
                cols: 120,
                rows: 50
            }]
        );
        assert_eq!(
            lifecycle_receiver.recv().await,
            Ok(PtyLifecycleEvent::Exited {
                session_id: PtySessionId::new("session-1"),
                exit_code: Some(0),
            })
        );
        assert_eq!(controls.kill_count(), 1);
    }

    #[derive(Clone)]
    struct FakePtyProcessFactory {
        output: String,
        controls: FakeProcessControls,
    }

    impl FakePtyProcessFactory {
        /// Builds a fake PTY process factory with deterministic startup output.
        fn with_output(output: &str) -> Self {
            Self {
                output: output.to_string(),
                controls: FakeProcessControls::default(),
            }
        }

        /// Returns the shared captured control state for assertions.
        fn controls(&self) -> FakeProcessControls {
            self.controls.clone()
        }
    }

    impl PtyProcessFactory for FakePtyProcessFactory {
        /// Starts a fake PTY process backed by in-memory reader and writer handles.
        fn spawn(
            &self,
            _request: PtyProcessSpawnRequest,
        ) -> Result<PtyProcess, PtyProcessFactoryError> {
            let (exit_sender, exit_receiver) = channel();

            Ok(PtyProcess {
                handle: Arc::new(FakeProcessHandle {
                    controls: self.controls.clone(),
                    exit_sender: Mutex::new(Some(exit_sender)),
                    exit_receiver: Mutex::new(exit_receiver),
                }),
                io: PtyIoHandle {
                    reader: Box::new(Cursor::new(self.output.clone().into_bytes())),
                    writer: Box::new(FakeWriter {
                        writes: self.controls.writes.clone(),
                    }),
                },
            })
        }
    }

    #[derive(Clone, Default)]
    struct FakeProcessControls {
        writes: Arc<Mutex<Vec<String>>>,
        resizes: Arc<Mutex<Vec<PtySize>>>,
        kill_count: Arc<Mutex<usize>>,
    }

    impl FakeProcessControls {
        /// Returns the collected input writes captured by the fake PTY writer.
        fn writes(&self) -> Vec<String> {
            self.writes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        /// Returns the collected resize requests captured by the fake PTY handle.
        fn resizes(&self) -> Vec<PtySize> {
            self.resizes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        /// Returns how many explicit or shutdown-driven kill requests were observed.
        fn kill_count(&self) -> usize {
            *self
                .kill_count
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    struct FakeProcessHandle {
        controls: FakeProcessControls,
        exit_sender: Mutex<Option<Sender<PtyChildExit>>>,
        exit_receiver: Mutex<Receiver<PtyChildExit>>,
    }

    impl PtyProcessHandle for FakeProcessHandle {
        /// Returns a stable fake process identifier for deterministic tests.
        fn id(&self) -> PtyProcessId {
            PtyProcessId::new(1)
        }

        /// Captures the resize request in memory for later assertions.
        fn resize(&self, size: PtySize) -> Result<(), PtyProcessFactoryError> {
            self.controls
                .resizes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(size);

            Ok(())
        }

        /// Captures the kill request and unblocks the waiting exit observer.
        fn kill(&self) -> Result<(), PtyProcessFactoryError> {
            *self
                .controls
                .kill_count
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            if let Some(sender) = self
                .exit_sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let _ = sender.send(PtyChildExit { exit_code: Some(0) });
            }

            Ok(())
        }

        /// Waits until the fake PTY has been killed and then returns the captured exit status.
        fn wait(&self) -> Result<PtyChildExit, PtyProcessFactoryError> {
            self.exit_receiver
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recv()
                .map_err(|error| PtyProcessFactoryError::WaitFailed {
                    message: error.to_string(),
                })
        }
    }

    struct FakeWriter {
        writes: Arc<Mutex<Vec<String>>>,
    }

    impl Write for FakeWriter {
        /// Appends every PTY input write into the shared test buffer.
        fn write(&mut self, buffer: &[u8]) -> IoResult<usize> {
            self.writes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(String::from_utf8_lossy(buffer).to_string());

            Ok(buffer.len())
        }

        /// Treats flush as a no-op because the fake writer has no external sink.
        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }
}
