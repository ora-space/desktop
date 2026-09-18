use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use ora_process_protocol::{HelperOperation, HelperRequest, HelperResponse, HelperStatus};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinSet;

use super::LinuxHelperConfig;

impl LinuxHelperConfig {
    /// Bounds active exchanges and cancels them on shutdown; the caller owns listener deployment.
    pub async fn serve_management(
        self,
        listener: UnixListener,
        shutdown: impl Future<Output = ()>,
    ) -> io::Result<()> {
        let config = Arc::new(self);
        let mut connections = JoinSet::new();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    connections.shutdown().await;
                    return Ok(());
                }
                result = connections.join_next(), if !connections.is_empty() => {
                    // Malformed, disconnected and timed-out peers affect only their exchange.
                    // A worker panic is instead a service failure and must remain visible.
                    if let Some(Err(error)) = result {
                        return Err(io::Error::other(error));
                    }
                }
                result = listener.accept(), if connections.len() < 16 => {
                    let (stream, _) = result?;
                    let config = Arc::clone(&config);
                    connections.spawn(async move { config.serve_management_connection(stream).await });
                }
            }
        }
    }

    /// Authenticates the kernel-reported connecting identity before decoding management input.
    ///
    /// This connection handler grants no launch authority and does not validate deployment.
    /// The service entry point must separately validate its administrator-owned configuration.
    pub async fn serve_management_connection(&self, mut stream: UnixStream) -> io::Result<()> {
        tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
            let status = if stream.peer_cred()?.uid() != self.0.manager_uid {
                HelperStatus::Unauthorized
            } else {
                let length = stream.read_u32().await?;
                if length > 16384 {
                    HelperStatus::InvalidRequest
                } else {
                    let mut bytes = vec![0; length as usize];
                    stream.read_exact(&mut bytes).await?;
                    match serde_json::from_slice::<HelperRequest>(&bytes) {
                        Ok(request) if request.version != 1 => HelperStatus::UnsupportedVersion,
                        Ok(HelperRequest {
                            operation: HelperOperation::Inspect,
                            ..
                        }) => HelperStatus::LaunchUnavailable,
                        Err(_) => HelperStatus::InvalidRequest,
                    }
                }
            };
            let reply = serde_json::to_vec(&HelperResponse { version: 1, status })?;
            stream.write_u32(reply.len() as u32).await?;
            stream.write_all(&reply).await?;
            Ok(())
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "helper request timed out"))?
    }
}
