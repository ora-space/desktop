use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const JSON_RPC_FRAME_TYPE: u8 = 0x01;
pub(crate) const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

/// Reads one length-delimited plugin protocol frame, returning `None` at a clean EOF.
pub(crate) async fn read_frame<R>(reader: &mut R) -> io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut length_bytes = [0_u8; 4];
    match reader.read_u8().await {
        Ok(first_byte) => length_bytes[0] = first_byte,
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    reader.read_exact(&mut length_bytes[1..]).await?;

    let length = u32::from_be_bytes(length_bytes) as usize;
    if !(1..=MAX_FRAME_LENGTH).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("plugin frame length {length} is outside the supported range"),
        ));
    }

    let mut frame = vec![0_u8; length];
    reader.read_exact(&mut frame).await?;
    if frame[0] != JSON_RPC_FRAME_TYPE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported plugin frame type {}", frame[0]),
        ));
    }

    Ok(Some(frame.split_off(1)))
}

/// Writes one JSON-RPC payload using Ora's binary plugin frame envelope.
pub(crate) async fn write_frame<W>(writer: &mut W, payload: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let length = payload
        .len()
        .checked_add(1)
        .filter(|length| *length <= MAX_FRAME_LENGTH)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "plugin frame is too large"))?;
    let length = u32::try_from(length)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "plugin frame is too large"))?;

    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_u8(JSON_RPC_FRAME_TYPE).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::{JSON_RPC_FRAME_TYPE, MAX_FRAME_LENGTH, read_frame, write_frame};
    use pretty_assertions::assert_eq;
    use tokio::io::{AsyncWriteExt, duplex};

    /// Verifies a payload survives a fragmented asynchronous frame round trip.
    #[tokio::test]
    async fn round_trips_json_rpc_frame() {
        let (mut writer, mut reader) = duplex(64);
        let payload = br#"{"jsonrpc":"2.0","id":1,"result":"ok"}"#.to_vec();
        let expected = payload.clone();
        let write_task = tokio::spawn(async move { write_frame(&mut writer, &payload).await });

        assert_eq!(read_frame(&mut reader).await.unwrap(), Some(expected));
        write_task.await.unwrap().unwrap();
    }

    /// Rejects unknown frame types instead of guessing how their payload should be handled.
    #[tokio::test]
    async fn rejects_unknown_frame_type() {
        let (mut writer, mut reader) = duplex(16);
        writer.write_all(&[0, 0, 0, 2, 0xff, b'{']).await.unwrap();

        assert_eq!(
            read_frame(&mut reader).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    /// Rejects lengths above the protocol ceiling before allocating their declared buffer.
    #[tokio::test]
    async fn rejects_oversized_frame() {
        let (mut writer, mut reader) = duplex(8);
        let length = u32::try_from(MAX_FRAME_LENGTH + 1).unwrap();
        writer.write_all(&length.to_be_bytes()).await.unwrap();
        writer.write_u8(JSON_RPC_FRAME_TYPE).await.unwrap();

        assert_eq!(
            read_frame(&mut reader).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    /// Treats a truncated header as corruption while preserving clean EOF as normal closure.
    #[tokio::test]
    async fn distinguishes_partial_header_from_clean_eof() {
        let (mut writer, mut reader) = duplex(8);
        writer.write_all(&[0, 0]).await.unwrap();
        writer.shutdown().await.unwrap();

        assert_eq!(
            read_frame(&mut reader).await.unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    }
}
