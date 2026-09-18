#![cfg(target_os = "linux")]

use ora_process_runtime::LinuxHelperConfig;
use pretty_assertions::assert_eq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::net::UnixStream;

/// Kernel peer identity, rather than any request payload, gates the privileged protocol.
#[tokio::test]
async fn unauthorized_connection_is_rejected_without_sending_a_request() {
    let (mut client, server) =
        UnixStream::pair().unwrap_or_else(|error| panic!("socket pair: {error:?}"));
    let actual_uid = server
        .peer_cred()
        .unwrap_or_else(|error| panic!("credentials: {error:?}"))
        .uid();
    let manager_uid = if actual_uid == 1000 { 1001 } else { 1000 };
    let config = LinuxHelperConfig::from_json(
        serde_json::json!({
            "version": 1, "manager_uid": manager_uid,
            "workload_uid": 2000, "workload_gid": 2000,
            "cgroup_root": "/sys/fs/cgroup/ora-workloads"
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap_or_else(|error| panic!("config: {error:?}"));
    let (result, response) = tokio::join!(config.serve_management_connection(server), async {
        let length = client
            .read_u32()
            .await
            .unwrap_or_else(|error| panic!("reply length: {error:?}"));
        let mut bytes = vec![0; length as usize];
        client
            .read_exact(&mut bytes)
            .await
            .unwrap_or_else(|error| panic!("reply body: {error:?}"));
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .unwrap_or_else(|error| panic!("reply JSON: {error:?}"))
    });
    result.unwrap_or_else(|error| panic!("handled rejection: {error:?}"));
    assert_eq!(
        response,
        serde_json::json!({"version":1,"status":"unauthorized"})
    );
}

/// An authenticated manager can inspect availability without obtaining execution authority.
#[tokio::test]
async fn manager_can_inspect_but_cannot_supply_launch_authority() {
    for (request, expected) in [
        (
            r#"{"version":1,"operation":"inspect"}"#,
            "launch_unavailable",
        ),
        (
            r#"{"version":2,"operation":"inspect"}"#,
            "unsupported_version",
        ),
        (
            r#"{"version":1,"operation":"exec","command":"/bin/true"}"#,
            "invalid_request",
        ),
        (
            r#"{"version":1,"operation":"inspect","manager_uid":0}"#,
            "invalid_request",
        ),
        ("not-json", "invalid_request"),
    ] {
        let (mut client, server) =
            UnixStream::pair().unwrap_or_else(|error| panic!("socket pair: {error:?}"));
        let manager_uid = server
            .peer_cred()
            .unwrap_or_else(|error| panic!("credentials: {error:?}"))
            .uid();
        // Root cannot be configured as the manager; positive tests need an unprivileged runner.
        assert!(
            manager_uid != 0,
            "positive authentication tests require a non-root runner"
        );
        let workload_uid = if manager_uid == 2000 { 2001 } else { 2000 };
        let config = LinuxHelperConfig::from_json(
            serde_json::json!({
                "version": 1, "manager_uid": manager_uid,
                "workload_uid": workload_uid, "workload_gid": 2000,
                "cgroup_root": "/sys/fs/cgroup/ora-workloads"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap_or_else(|error| panic!("config: {error:?}"));
        let (result, response) = tokio::join!(config.serve_management_connection(server), async {
            client
                .write_u32(request.len() as u32)
                .await
                .unwrap_or_else(|error| panic!("length: {error:?}"));
            client
                .write_all(request.as_bytes())
                .await
                .unwrap_or_else(|error| panic!("request: {error:?}"));
            let length = client
                .read_u32()
                .await
                .unwrap_or_else(|error| panic!("reply length: {error:?}"));
            let mut bytes = vec![0; length as usize];
            client
                .read_exact(&mut bytes)
                .await
                .unwrap_or_else(|error| panic!("reply body: {error:?}"));
            let mut tail = Vec::new();
            client
                .read_to_end(&mut tail)
                .await
                .unwrap_or_else(|error| panic!("connection closed: {error:?}"));
            assert_eq!(tail, Vec::<u8>::new());
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .unwrap_or_else(|error| panic!("reply JSON: {error:?}"))
        });
        result.unwrap_or_else(|error| panic!("handled request: {error:?}"));
        assert_eq!(response, serde_json::json!({"version":1,"status":expected}));
    }
}

/// The length prefix is bounded before allocation, and a partial request cannot pin a worker.
#[tokio::test]
async fn oversized_and_stalled_requests_are_bounded() {
    for length in [u32::MAX, 16] {
        let (mut client, server) =
            UnixStream::pair().unwrap_or_else(|error| panic!("socket pair: {error:?}"));
        let manager_uid = server
            .peer_cred()
            .unwrap_or_else(|error| panic!("credentials: {error:?}"))
            .uid();
        assert!(
            manager_uid != 0,
            "positive authentication tests require a non-root runner"
        );
        let workload_uid = if manager_uid == 2000 { 2001 } else { 2000 };
        let config = LinuxHelperConfig::from_json(
            serde_json::json!({
                "version": 1, "manager_uid": manager_uid,
                "workload_uid": workload_uid, "workload_gid": 2000,
                "cgroup_root": "/sys/fs/cgroup/ora-workloads"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap_or_else(|error| panic!("config: {error:?}"));
        client
            .write_u32(length)
            .await
            .unwrap_or_else(|error| panic!("length: {error:?}"));
        let result = config.serve_management_connection(server).await;
        if length == u32::MAX {
            result.unwrap_or_else(|error| panic!("handled oversized request: {error:?}"));
            let length = client
                .read_u32()
                .await
                .unwrap_or_else(|error| panic!("reply length: {error:?}"));
            let mut bytes = vec![0; length as usize];
            client
                .read_exact(&mut bytes)
                .await
                .unwrap_or_else(|error| panic!("reply body: {error:?}"));
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .unwrap_or_else(|error| panic!("JSON: {error:?}")),
                serde_json::json!({"version":1,"status":"invalid_request"})
            );
        } else {
            assert_eq!(
                result
                    .err()
                    .unwrap_or_else(|| panic!("stalled request: expected an error"))
                    .kind(),
                std::io::ErrorKind::TimedOut
            );
        }
    }
}

/// Shutdown cancels incomplete exchanges rather than waiting for clients to finish their frames.
#[tokio::test]
async fn management_service_accepts_connections_and_shutdown_releases_them() {
    let directory =
        tempfile::tempdir().unwrap_or_else(|error| panic!("temporary directory: {error:?}"));
    let path = directory.path().join("helper.sock");
    let listener = UnixListener::bind(&path).unwrap_or_else(|error| panic!("listener: {error:?}"));
    let mut client = UnixStream::connect(&path)
        .await
        .unwrap_or_else(|error| panic!("connect: {error:?}"));
    let manager_uid = client
        .peer_cred()
        .unwrap_or_else(|error| panic!("credentials: {error:?}"))
        .uid();
    assert!(
        manager_uid != 0,
        "positive authentication tests require a non-root runner"
    );
    let workload_uid = if manager_uid == 2000 { 2001 } else { 2000 };
    let config = LinuxHelperConfig::from_json(
        serde_json::json!({
            "version": 1, "manager_uid": manager_uid,
            "workload_uid": workload_uid, "workload_gid": 2000,
            "cgroup_root": "/sys/fs/cgroup/ora-workloads"
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap_or_else(|error| panic!("config: {error:?}"));
    let request = br#"{"version":1,"operation":"inspect"}"#;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let (result, ()) = tokio::join!(
        config.serve_management(listener, async {
            let _ = stopped.await;
        }),
        async {
            client
                .write_u32(request.len() as u32)
                .await
                .unwrap_or_else(|error| panic!("length: {error:?}"));
            client
                .write_all(request)
                .await
                .unwrap_or_else(|error| panic!("request: {error:?}"));
            let length = client
                .read_u32()
                .await
                .unwrap_or_else(|error| panic!("reply length: {error:?}"));
            let mut bytes = vec![0; length as usize];
            client
                .read_exact(&mut bytes)
                .await
                .unwrap_or_else(|error| panic!("reply body: {error:?}"));
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .unwrap_or_else(|error| panic!("JSON: {error:?}")),
                serde_json::json!({"version":1,"status":"launch_unavailable"})
            );
            let mut stalled = UnixStream::connect(&path)
                .await
                .unwrap_or_else(|error| panic!("stalled connect: {error:?}"));
            stop.send(())
                .unwrap_or_else(|error| panic!("shutdown: {error:?}"));
            let mut bytes = Vec::new();
            // A queued connection may be reset, while an accepted idle connection sees EOF.
            let outcome = stalled.read_to_end(&mut bytes).await;
            assert!(
                outcome.is_ok()
                    || outcome
                        .err()
                        .unwrap_or_else(|| panic!("reset: expected an error"))
                        .kind()
                        == std::io::ErrorKind::ConnectionReset
            );
            assert_eq!(bytes, Vec::<u8>::new());
        }
    );
    result.unwrap_or_else(|error| panic!("service shutdown: {error:?}"));
}

/// Failed deployment authorization cannot unlink or replace an existing endpoint path.
#[tokio::test]
async fn deployment_failure_preserves_existing_endpoint() {
    let directory =
        tempfile::tempdir().unwrap_or_else(|error| panic!("temporary directory: {error:?}"));
    let endpoint = directory.path().join("helper.sock");
    std::fs::write(&endpoint, b"do not replace")
        .unwrap_or_else(|error| panic!("existing file: {error:?}"));
    let result = ora_process_runtime::serve_linux_helper(
        &directory.path().join("missing-config.json"),
        &endpoint,
        std::future::pending(),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(&endpoint).unwrap_or_else(|error| panic!("preserved endpoint: {error:?}")),
        b"do not replace"
    );
}
