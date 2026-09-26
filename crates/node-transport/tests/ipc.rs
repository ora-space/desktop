#![cfg(unix)]
#![allow(clippy::unwrap_used)]
//! Local IPC framing through the production halves and acceptor.
use ora_node_protocol::{FrameError, MAX_FRAME_LENGTH, NODE_MESSAGE_FRAME_TYPE};
use ora_node_transport::{
    Acceptor, CloseReason, ConnectFailure, FrameReceiver, FrameSender, TransportError,
    close_connection, ipc,
};
use pretty_assertions::assert_eq;
use std::time::Duration;
use tokio::{io::AsyncWriteExt, net::UnixStream};

/// Builds the legacy stream bytes for one body so the test pins the unchanged wire format.
fn wire(body: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(body);
    bytes
}

/// Frame halves carry bodies over exactly the length-prefixed bytes the stream codec uses.
#[tokio::test]
async fn frames_round_trip_with_length_prefix_wire() {
    let body = vec![NODE_MESSAGE_FRAME_TYPE, b'{', b'}'];

    let (left, mut raw) = UnixStream::pair().unwrap();
    let (_, mut sender) = ipc::split(left);
    sender.send(body.clone()).await.unwrap();
    let mut received = vec![0; wire(&body).len()];
    tokio::io::AsyncReadExt::read_exact(&mut raw, &mut received)
        .await
        .unwrap();
    assert_eq!(received, wire(&body));

    let (left, mut raw) = UnixStream::pair().unwrap();
    let (mut receiver, _) = ipc::split(left);
    raw.write_all(&wire(&body)).await.unwrap();
    assert_eq!(receiver.recv().await.unwrap(), Some(body));
    drop(raw);
    assert_eq!(receiver.recv().await.unwrap(), None);
}

/// IPC has no close code: a deliberate close is a clean end of stream for the peer, and
/// `close_connection` returns once the peer has ended its side too.
#[tokio::test]
async fn deliberate_close_is_a_clean_end_of_stream() {
    let (left, right) = UnixStream::pair().unwrap();
    let (mut receiver, mut sender) = ipc::split(left);
    let (mut peer_receiver, peer_sender) = ipc::split(right);
    let peer = async move {
        let end = peer_receiver.recv().await.unwrap();
        drop(peer_sender);
        end
    };
    let (end, ()) = tokio::join!(
        peer,
        close_connection(&mut receiver, &mut sender, CloseReason::IdentityMismatch)
    );
    assert_eq!(end, None);
}

/// Dropping a pending receive mid-frame, as a `select!` does, neither loses nor splits the frame.
#[tokio::test]
async fn cancelled_receive_keeps_partial_frame() {
    let (left, mut right) = UnixStream::pair().unwrap();
    let (mut receiver, _) = ipc::split(left);
    let body = vec![NODE_MESSAGE_FRAME_TYPE; 64];
    let bytes = wire(&body);
    let writer = tokio::spawn(async move {
        for chunk in bytes.chunks(3) {
            right.write_all(chunk).await.unwrap();
            tokio::time::sleep(Duration::from_millis(/*millis*/ 2)).await;
        }
        right
    });
    let received = loop {
        tokio::select! {
            frame = receiver.recv() => break frame.unwrap(),
            () = tokio::time::sleep(Duration::from_millis(/*millis*/ 1)) => {}
        }
    };
    assert_eq!(received, Some(body));
    drop(writer.await.unwrap());
    assert_eq!(receiver.recv().await.unwrap(), None);
}

/// Truncated and out-of-range frames are errors, never a clean end or a shortened frame.
#[tokio::test]
async fn rejects_truncated_and_out_of_range_frames() {
    let (left, mut right) = UnixStream::pair().unwrap();
    let (mut receiver, _) = ipc::split(left);
    right
        .write_all(&[0, 0, 0, 4, NODE_MESSAGE_FRAME_TYPE])
        .await
        .unwrap();
    drop(right);
    assert!(matches!(receiver.recv().await, Err(TransportError::Io(_))));

    for length in [0, MAX_FRAME_LENGTH + 1] {
        let (left, mut right) = UnixStream::pair().unwrap();
        let (mut receiver, _) = ipc::split(left);
        right
            .write_all(&u32::try_from(length).unwrap().to_be_bytes())
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Err(TransportError::Frame(FrameError::InvalidLength { length: actual })) if actual == length
        ));
    }
}

/// A missing endpoint is unreachable; a busy rejection closes the socket so the peer sees EOF.
#[tokio::test]
async fn connect_classifies_missing_endpoint_and_busy_rejection_closes() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = root.path().join("control.sock");
    let error = ipc::connect(&endpoint).await.err().unwrap();
    assert_eq!(error.failure, ConnectFailure::Unreachable);

    let acceptor = ipc::IpcAcceptor::new(tokio::net::UnixListener::bind(&endpoint).unwrap());
    let (connected, accepted) = tokio::join!(ipc::connect(&endpoint), acceptor.accept());
    let (mut receiver, _) = connected.unwrap();
    acceptor.reject_busy(accepted.unwrap()).await;
    assert_eq!(receiver.recv().await.unwrap(), None);
}
