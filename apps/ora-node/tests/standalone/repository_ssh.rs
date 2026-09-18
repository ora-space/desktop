use super::*;
use crate::support::{ChildGuard, until};
use std::{
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
};

/// Proves real explicit SSH acquisition and host-key rejection using an unprivileged fixture daemon.
#[test]
fn ssh_clone_requires_deployment_identity_and_known_host() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let root = fixture.path();
        let host_key = root.join("host-key");
        let client_key = root.join("client-key");
        for key in [&host_key, &client_key] {
            let output = Command::new("/usr/bin/ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(key)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let username = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned();
        let daemon_config = root.join("sshd_config");
        fs::write(&daemon_config, format!("Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\nStrictModes yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nAllowUsers {username}\nLogLevel ERROR\n", host_key.display(), root.join("sshd.pid").display(), client_key.with_extension("pub").display())).unwrap();
        let mut daemon = ChildGuard(
            Command::new("/usr/sbin/sshd")
                .args(["-D", "-e", "-f"])
                .arg(&daemon_config)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("install openssh-server for Linux clone acceptance"),
        );
        until(|| {
            assert!(daemon.0.try_wait().unwrap().is_none());
            TcpStream::connect(("127.0.0.1", port)).is_ok()
        });
        let server = HttpsRepository::new(root, root.join("main").join(".git"));
        let mut config = configuration(&fixture, &server);
        let known_hosts = root.join("known_hosts");
        fs::write(&known_hosts, "").unwrap();
        let client_config = root.join("ssh_config");
        fs::write(&client_config, format!("Host *\n IdentityFile {}\n IdentitiesOnly yes\n UserKnownHostsFile {}\n GlobalKnownHostsFile /dev/null\n", client_key.display(), known_hosts.display())).unwrap();
        fs::set_permissions(&client_config, fs::Permissions::from_mode(/*mode*/ 0o600)).unwrap();
        config.ssh = CloneSsh::Configured {
            program: PathBuf::from("/usr/bin/ssh"),
            config: client_config,
        };
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        let mut unknown = request(&server, "unknown-host", "main");
        unknown.payload.spec.repository = CloneRepositoryUrl::parse(&format!(
            "ssh://{username}@127.0.0.1:{port}{}",
            root.join("main").display()
        ))
        .unwrap();
        let result = node.submit_clone(unknown.clone()).unwrap();
        assert!(matches!(
            result.state,
            ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneFailed(_)))
        ));
        let public_key = fs::read_to_string(host_key.with_extension("pub")).unwrap();
        fs::write(&known_hosts, format!("[127.0.0.1]:{port} {public_key}")).unwrap();
        let mut accepted = request(&server, "trusted-host", "main");
        accepted.payload.spec.repository = unknown.payload.spec.repository;
        let result = node.submit_clone(accepted).unwrap();
        assert!(
            matches!(
                result.state,
                ExecutionState::Completed(ExecutionResult::Clone(
                    CloneExecutionResult::CloneReady(_)
                ))
            ),
            "{result:?}"
        );
    });
}
