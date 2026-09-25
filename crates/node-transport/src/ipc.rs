//! Local IPC over a Unix socket. Each frame body travels behind a four-byte big-endian length, the
//! same bytes the protocol's stream reader and writer use, so the wire format is unchanged.
use crate::{Acceptor, ConnectError, ConnectFailure, FrameReceiver, FrameSender, TransportError};
use futures_util::{SinkExt, StreamExt};
use ora_node_protocol::{FrameError, MAX_FRAME_LENGTH};
use std::{future::Future, io, path::Path};
use tokio::net::{
    UnixListener, UnixStream,
    unix::{OwnedReadHalf, OwnedWriteHalf},
};
use tokio_util::{
    bytes::{Buf, BufMut, BytesMut},
    codec::{Decoder, Encoder, FramedRead, FramedWrite},
};

const LENGTH_PREFIX: usize = 4;

/// Length-prefixed framing. Decoding keeps partial input in the framed buffer between polls, which
/// is what makes [`IpcReceiver::recv`] cancel-safe.
#[derive(Default)]
struct IpcFrameCodec;

impl Decoder for IpcFrameCodec {
    type Item = Vec<u8>;
    type Error = TransportError;

    /// Rejects an out-of-range declared length before buffering its body.
    fn decode(&mut self, source: &mut BytesMut) -> Result<Option<Vec<u8>>, TransportError> {
        let Some(prefix) = source.get(..LENGTH_PREFIX) else {
            return Ok(None);
        };
        let mut length_bytes = [0_u8; LENGTH_PREFIX];
        length_bytes.copy_from_slice(prefix);
        let length = u32::from_be_bytes(length_bytes) as usize;
        if !(1..=MAX_FRAME_LENGTH).contains(&length) {
            return Err(FrameError::InvalidLength { length }.into());
        }
        if source.len() < LENGTH_PREFIX + length {
            source.reserve(LENGTH_PREFIX + length - source.len());
            return Ok(None);
        }
        source.advance(LENGTH_PREFIX);
        Ok(Some(source.split_to(length).to_vec()))
    }
}

impl Encoder<Vec<u8>> for IpcFrameCodec {
    type Error = TransportError;

    /// Bodies come from the protocol encoder, which already bounds them; the check keeps a
    /// malformed caller from emitting a prefix the reader would reject.
    fn encode(&mut self, frame: Vec<u8>, target: &mut BytesMut) -> Result<(), TransportError> {
        let length = u32::try_from(frame.len())
            .ok()
            .filter(|length| (1..=MAX_FRAME_LENGTH).contains(&(*length as usize)))
            .ok_or(FrameError::InvalidLength {
                length: frame.len(),
            })?;
        target.reserve(LENGTH_PREFIX + frame.len());
        target.put_u32(length);
        target.extend_from_slice(&frame);
        Ok(())
    }
}

/// Receiving half of a local IPC connection.
pub struct IpcReceiver(FramedRead<OwnedReadHalf, IpcFrameCodec>);

/// Sending half of a local IPC connection.
pub struct IpcSender(FramedWrite<OwnedWriteHalf, IpcFrameCodec>);

impl FrameReceiver for IpcReceiver {
    /// EOF between frames is a clean end; EOF inside a frame is an I/O error.
    async fn recv(&mut self) -> Result<Option<Vec<u8>>, TransportError> {
        self.0.next().await.transpose()
    }
}

impl FrameSender for IpcSender {
    /// Writes and flushes one length-prefixed frame.
    fn send(&mut self, frame: Vec<u8>) -> impl Future<Output = Result<(), TransportError>> + Send {
        self.0.send(frame)
    }
}

/// Splits a connected socket into frame halves.
pub fn split(stream: UnixStream) -> (IpcReceiver, IpcSender) {
    let (reader, writer) = stream.into_split();
    (
        IpcReceiver(FramedRead::new(reader, IpcFrameCodec)),
        IpcSender(FramedWrite::new(writer, IpcFrameCodec)),
    )
}

/// Connects to a Node's local endpoint. Every failure is `Unreachable`: a missing or refusing
/// socket only means no Node is listening right now.
pub async fn connect(endpoint: &Path) -> Result<(IpcReceiver, IpcSender), ConnectError> {
    UnixStream::connect(endpoint)
        .await
        .map(split)
        .map_err(|error| ConnectError::new(ConnectFailure::Unreachable, error))
}

/// Accepts control connections on an already bound private socket; endpoint creation and
/// permissions stay with the caller's deployment code.
pub struct IpcAcceptor(UnixListener);

impl IpcAcceptor {
    pub fn new(listener: UnixListener) -> Self {
        Self(listener)
    }
}

impl Acceptor for IpcAcceptor {
    type Pending = UnixStream;
    type Receiver = IpcReceiver;
    type Sender = IpcSender;

    /// Waits for the next local connection.
    async fn accept(&self) -> io::Result<UnixStream> {
        self.0.accept().await.map(|(stream, _)| stream)
    }

    /// A Unix socket has no transport handshake.
    fn open(
        &self,
        pending: UnixStream,
    ) -> impl Future<Output = Result<(IpcReceiver, IpcSender), TransportError>> + Send {
        std::future::ready(Ok(split(pending)))
    }

    /// Closing the socket is the only refusal IPC can express; the Controller sees EOF.
    fn reject_busy(&self, pending: UnixStream) -> impl Future<Output = ()> + Send + 'static {
        drop(pending);
        std::future::ready(())
    }
}
