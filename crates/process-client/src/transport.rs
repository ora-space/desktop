use ora_process_protocol::{
    GUARDIAN_MAX_FRAME, GuardianAccess, GuardianChannel, decode_guardian_payload,
    encode_guardian_frame,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

/// Shares OS authentication, bounded framing and exchange deadlines across guardian capabilities.
pub(super) async fn exchange<Request: Serialize, Reply: DeserializeOwned>(
    access: &GuardianAccess,
    expected_uid: u32,
    channel: GuardianChannel,
    request: &Request,
) -> io::Result<Reply> {
    exchange_path(
        &access.scope_dir.join(channel.socket_name()),
        expected_uid,
        request,
    )
    .await
}

/// Shares bounded local framing between host and guardian without sharing their authority models.
pub(super) async fn exchange_path<Request: Serialize, Reply: DeserializeOwned>(
    endpoint: &std::path::Path,
    expected_uid: u32,
    request: &Request,
) -> io::Result<Reply> {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
        let mut stream = UnixStream::connect(endpoint).await?;
        if stream.peer_cred()?.uid() != expected_uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "guardian OS identity mismatch",
            ));
        }
        stream.write_all(&encode_guardian_frame(request)?).await?;
        let length = stream.read_u32().await? as usize;
        if length > GUARDIAN_MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guardian frame exceeds limit",
            ));
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await?;
        decode_guardian_payload(&bytes)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "guardian exchange timed out"))?
}
