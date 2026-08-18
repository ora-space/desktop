use std::future::Future;

use futures_util::StreamExt;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

use crate::error::AcpError;

pub(crate) const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Yields whole inbound ACP messages, or the framing failure that ended the connection.
///
/// The stream is unbounded because it is connection-wide: bounding it would let one busy session
/// stall every other session sharing the same agent process.
pub type AcpMessages = mpsc::UnboundedReceiver<Result<Value, AcpError>>;

/// Carries whole ACP JSON-RPC messages for one connection.
///
/// Implementations own framing and ordering: `send` must deliver complete messages in call order,
/// and the receiver handed to `AcpPeer::spawn` must yield exactly one message per frame. The peer
/// never inspects transport-level framing, so a transport may be a byte stream (NDJSON over stdio)
/// or an already-parsed channel (plugin IPC).
pub trait AcpTransport: Send + Sync + 'static {
    fn send(&self, message: Value) -> impl Future<Output = Result<(), AcpError>> + Send;
}

/// Carries ACP messages as newline-delimited JSON over one child process's stdio pipes.
pub struct NdjsonTransport<Writer> {
    writer: AsyncMutex<Writer>,
}

impl<Writer> NdjsonTransport<Writer>
where
    Writer: AsyncWrite + Unpin + Send + 'static,
{
    /// Starts line decoding on `reader` and pairs the sink half with its inbound message stream.
    pub fn spawn<Reader>(reader: Reader, writer: Writer) -> (Self, AcpMessages)
    where
        Reader: AsyncRead + Unpin + Send + 'static,
    {
        let (sender, messages) = mpsc::unbounded_channel();
        tokio::spawn(decode_lines(reader, sender));
        (
            Self {
                writer: AsyncMutex::new(writer),
            },
            messages,
        )
    }
}

impl<Writer> AcpTransport for NdjsonTransport<Writer>
where
    Writer: AsyncWrite + Unpin + Send + 'static,
{
    /// Serializes one complete NDJSON line so concurrent writers cannot interleave bytes.
    async fn send(&self, message: Value) -> Result<(), AcpError> {
        let mut bytes = serde_json::to_vec(&message)
            .map_err(|error| AcpError::InvalidFrame(error.to_string()))?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(AcpError::FrameTooLarge);
        }
        bytes.push(b'\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(&bytes).await?;
        writer.flush().await?;
        Ok(())
    }
}

/// Decodes one byte stream into whole JSON values until EOF or the first framing failure.
///
/// A framing failure is terminal: the reader cannot know where the next message begins, so it
/// forwards the failure once and stops rather than resynchronizing on arbitrary bytes.
async fn decode_lines<Reader>(
    reader: Reader,
    sender: mpsc::UnboundedSender<Result<Value, AcpError>>,
) where
    Reader: AsyncRead + Unpin,
{
    let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(MAX_FRAME_BYTES));
    while let Some(line) = lines.next().await {
        let message = match line {
            Ok(line) => serde_json::from_str::<Value>(&line)
                .map_err(|error| AcpError::InvalidFrame(error.to_string())),
            Err(LinesCodecError::MaxLineLengthExceeded) => Err(AcpError::FrameTooLarge),
            Err(LinesCodecError::Io(error)) => Err(AcpError::Io(error)),
        };
        let terminal = message.is_err();
        if sender.send(message).is_err() || terminal {
            return;
        }
    }
}
