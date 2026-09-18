use super::*;
use pretty_assertions::assert_eq;

/// Clone roots never share a state tree, an existing checkout, or its parent/child namespace.
#[test]
fn clone_configuration_rejects_protected_roots_without_changing_permissions() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        let child = fixture.config().home_directory.join("repositories");
        fs::create_dir(&child).unwrap();
        fs::set_permissions(&child, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
        for root in [
            fixture.path().to_path_buf(),
            fixture.config().home_directory,
            fixture.process().host_directory,
            child,
        ] {
            let before = fs::metadata(&root).unwrap().permissions();
            let mut invalid = config.clone();
            invalid.repository_root = root.clone();
            assert!(node.configure_clone(invalid).is_err());
            assert_eq!(fs::metadata(root).unwrap().permissions(), before);
        }
        node.configure_clone(config).unwrap();
    });
}

/// Deployment credentials work while hooks, LFS downloads and recursive submodule acquisition stay off.
#[test]
fn private_https_uses_deployment_credentials_without_running_checkout_extensions() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let main = fixture.path().join("main");
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:0000000000000000000000000000000000000000000000000000000000000000\nsize 123\n";
        fs::write(main.join("large.bin"), pointer).unwrap();
        fs::write(
            main.join(".gitattributes"),
            "*.bin filter=lfs diff=lfs merge=lfs -text\n",
        )
        .unwrap();
        fs::write(main.join(".gitmodules"), "[submodule \"nested\"]\n path = nested\n url = https://invalid.example.test/must-not-fetch\n").unwrap();
        let base = fixture.git(&["rev-parse", "HEAD"]);
        fixture.git(&["add", ".gitattributes", "large.bin", ".gitmodules"]);
        fixture.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{base},nested"),
        ]);
        fixture.git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "extensions",
        ]);
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), main.join(".git"));
        server.require_credentials.store(true, Ordering::SeqCst);
        let config = configuration(&fixture, &server);
        let hooks = fixture.path().join("configured-hooks");
        fs::create_dir(&hooks).unwrap();
        let marker = fixture.path().join("extension-ran");
        let extension = format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display());
        for path in [hooks.join("post-checkout"), fixture.path().join("smudge")] {
            fs::write(&path, &extension).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
        }
        let credentials = fixture.path().join("credentials");
        fs::write(&credentials, "#!/bin/sh\nif test \"$1\" = get; then printf 'username=fixture\\npassword=secret\\n'; fi\n").unwrap();
        fs::set_permissions(&credentials, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
        fs::write(&config.git_config, format!("[http]\n sslCAInfo = {}\n[credential]\n helper = {}\n[core]\n hooksPath = {}\n[filter \"lfs\"]\n required = true\n smudge = {}\n process = {}\n[submodule]\n recurse = true\n", server.certificate.display(), credentials.display(), hooks.display(), fixture.path().join("smudge").display(), fixture.path().join("smudge").display())).unwrap();
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        let result = node
            .submit_clone(request(&server, "private", "main"))
            .unwrap();
        let ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneReady(
            ready,
        ))) = result.state
        else {
            panic!("private clone not ready: {result:?}");
        };
        let target = PathBuf::from(ready.path.as_str());
        assert_eq!(
            fs::read_to_string(target.join("large.bin")).unwrap(),
            pointer
        );
        assert!(!target.join("nested").join(".git").exists());
        assert!(!marker.exists());
        drop(node);
        for path in [
            fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.path().join("host").join("host.sqlite"),
        ] {
            let bytes = fs::read(path).unwrap();
            assert!(
                !bytes
                    .windows(b"password=secret".len())
                    .any(|window| window == b"password=secret")
            );
            assert!(
                !bytes
                    .windows(b"Zml4dHVyZTpzZWNyZXQ=".len())
                    .any(|window| window == b"Zml4dHVyZTpzZWNyZXQ=")
            );
        }
    });
}
