use pretty_assertions::assert_eq;
use std::process::Command;

/// A deployment executable must not silently accept arbitrary commands or claim launch readiness.
#[test]
fn helper_cli_rejects_execution_and_missing_deployments() {
    for arguments in [
        vec!["--exec", "/bin/true"],
        vec!["--check"],
        vec!["--check", "relative-config.json"],
        vec!["--serve"],
        vec!["--serve", "relative-config.json", "relative.sock"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_ora-process-helper"))
            .args(arguments)
            .output()
            .unwrap_or_else(|error| panic!("helper: {error}"));
        assert_eq!(result.status.success(), false);
        assert_eq!(result.stdout, Vec::<u8>::new());
    }
}
