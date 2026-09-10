use crate::message::ValidateMessage;
use crate::{ControllerToNodeMessage, MessageValidationError, NodeToControllerMessage};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Frame tag identifying a Controller–Node JSON envelope.
pub const NODE_MESSAGE_FRAME_TYPE: u8 = 0x01;
/// Largest complete Controller–Node frame, including its one-byte type tag.
pub const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

/// Distinguishes transport truncation, framing failures, JSON failures, and protocol validation.
#[derive(Debug, Error)]
pub enum FrameError {
    #[error("Controller–Node frame I/O failed")]
    Io(#[from] io::Error),
    #[error("Controller–Node frame length {length} is outside the supported range")]
    InvalidLength { length: usize },
    #[error("unsupported Controller–Node frame type {frame_type}")]
    UnsupportedFrameType { frame_type: u8 },
    #[error("failed to encode Controller–Node message as JSON")]
    EncodeJson(#[source] serde_json::Error),
    #[error("failed to decode Controller–Node message JSON")]
    DecodeJson(#[source] serde_json::Error),
    #[error(transparent)]
    InvalidMessage(#[from] MessageValidationError),
}

/// Reads and validates one message sent from a Controller, returning `None` only at clean EOF.
pub async fn read_controller_message<R>(
    reader: &mut R,
) -> Result<Option<ControllerToNodeMessage>, FrameError>
where
    R: AsyncRead + Unpin,
{
    read_message(reader).await
}

/// Encodes and writes one validated message sent from a Controller.
pub async fn write_controller_message<W>(
    writer: &mut W,
    message: &ControllerToNodeMessage,
) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
{
    write_message(writer, message).await
}

/// Reads and validates one message sent from a Node, returning `None` only at clean EOF.
pub async fn read_node_message<R>(
    reader: &mut R,
) -> Result<Option<NodeToControllerMessage>, FrameError>
where
    R: AsyncRead + Unpin,
{
    read_message(reader).await
}

/// Encodes and writes one validated message sent from a Node.
pub async fn write_node_message<W>(
    writer: &mut W,
    message: &NodeToControllerMessage,
) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
{
    write_message(writer, message).await
}

/// Shares decoding and invariant checks while public functions retain peer direction in their type.
async fn read_message<R, Message>(reader: &mut R) -> Result<Option<Message>, FrameError>
where
    R: AsyncRead + Unpin,
    Message: DeserializeOwned + ValidateMessage,
{
    let Some(payload) = read_frame(reader).await? else {
        return Ok(None);
    };
    // TODO(todo-87602f0b): WARN on decode rejection with receive direction, payload length,
    // NODE_MESSAGE_FRAME_TYPE and JSON error category/location; no typed value exists yet.
    let message = serde_json::from_slice::<Message>(&payload).map_err(FrameError::DecodeJson)?;
    // TODO(todo-87602f0b): WARN on inbound validation rejection with receive direction,
    // payload length, frame type and MessageValidationError category/fields.
    message.validate()?;
    // TODO(todo-87602f0b): TRACE the accepted inbound typed message, receive direction,
    // payload length and frame type here, after all validation succeeds.
    Ok(Some(message))
}

/// Shares encoding and invariant checks while public functions retain peer direction in their type.
async fn write_message<W, Message>(writer: &mut W, message: &Message) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    Message: Serialize + ValidateMessage,
{
    // TODO(todo-87602f0b): WARN on outbound validation rejection with send direction and
    // MessageValidationError category/fields; no serialized payload or frame length exists yet.
    message.validate()?;
    // TODO(todo-87602f0b): ERROR on JSON encode failure with send direction and serializer
    // error category; the typed message is available but no complete payload length exists yet.
    let payload = serde_json::to_vec(message).map_err(FrameError::EncodeJson)?;
    // TODO(todo-87602f0b): TRACE the outbound typed message, send direction, payload length
    // and frame type after this write succeeds; failures are recorded at their I/O sites below.
    write_frame(writer, &payload).await
}

/// Reads framing metadata before allocating the bounded JSON payload.
async fn read_frame<R>(reader: &mut R) -> Result<Option<Vec<u8>>, FrameError>
where
    R: AsyncRead + Unpin,
{
    let mut length_bytes = [0_u8; 4];
    match reader.read_u8().await {
        Ok(first_byte) => length_bytes[0] = first_byte,
        // TODO(todo-87602f0b): TRACE clean receive EOF here; no frame metadata is available.
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        // TODO(todo-87602f0b): WARN on receive header-start I/O failure with error.kind();
        // no complete frame length or type is available.
        Err(error) => return Err(FrameError::Io(error)),
    }
    // TODO(todo-87602f0b): WARN on receive length-header I/O failure with first byte and
    // error.kind(); remaining length bytes may be incomplete and must not be treated as a length.
    reader.read_exact(&mut length_bytes[1..]).await?;

    let length = u32::from_be_bytes(length_bytes) as usize;
    if !(1..=MAX_FRAME_LENGTH).contains(&length) {
        // TODO(todo-87602f0b): WARN on rejected receive length with length, MAX_FRAME_LENGTH
        // and InvalidLength category; frame type has not been read.
        return Err(FrameError::InvalidLength { length });
    }

    // TODO(todo-87602f0b): WARN on receive type-header I/O failure with length and error.kind().
    let frame_type = reader.read_u8().await?;
    if frame_type != NODE_MESSAGE_FRAME_TYPE {
        // TODO(todo-87602f0b): WARN on rejected receive type with length, frame_type and
        // UnsupportedFrameType category; no typed value or payload has been decoded.
        return Err(FrameError::UnsupportedFrameType { frame_type });
    }

    let payload_length = length - 1;
    let mut payload = vec![0_u8; payload_length];
    // TODO(todo-87602f0b): WARN on receive payload I/O failure with length, frame_type,
    // expected payload_length and error.kind(); the buffer may contain only partial data.
    reader.read_exact(&mut payload).await?;
    Ok(Some(payload))
}

/// Writes one validated JSON payload using the protocol's binary frame envelope.
async fn write_frame<W>(writer: &mut W, payload: &[u8]) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
{
    // TODO(todo-87602f0b): WARN on outbound length rejection with payload.len(),
    // MAX_FRAME_LENGTH, NODE_MESSAGE_FRAME_TYPE and InvalidLength category before writing bytes.
    let length = payload
        .len()
        .checked_add(/*rhs*/ 1)
        .filter(|length| *length <= MAX_FRAME_LENGTH)
        .ok_or(FrameError::InvalidLength {
            length: payload.len().saturating_add(/*rhs*/ 1),
        })?;
    // TODO(todo-87602f0b): ERROR if the bounded outbound length cannot fit u32, recording
    // length, frame type and InvalidLength category (currently prevented by MAX_FRAME_LENGTH).
    let length = u32::try_from(length).map_err(|_| FrameError::InvalidLength { length })?;

    // TODO(todo-87602f0b): WARN on send length-header I/O failure with length, frame type
    // and error.kind(); write_all may already have emitted part of the header.
    writer.write_all(&length.to_be_bytes()).await?;
    // TODO(todo-87602f0b): WARN on send type-header I/O failure with length,
    // NODE_MESSAGE_FRAME_TYPE and error.kind().
    writer.write_u8(NODE_MESSAGE_FRAME_TYPE).await?;
    // TODO(todo-87602f0b): WARN on send payload I/O failure with length, frame type,
    // payload.len() and error.kind(); partial writes cannot be rolled back.
    writer.write_all(payload).await?;
    // TODO(todo-87602f0b): WARN on send flush I/O failure with length, frame type and
    // error.kind(); successful writes do not imply flush or peer acceptance succeeded.
    writer.flush().await?;
    Ok(())
}
