#![cfg(unix)]
//! How a host judges a shutdown the reaper did not acknowledge. This file is its own test binary
//! because the reaper client is process-global and can be started only once.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// A reaper that read the shutdown request and exited successfully without acknowledging it, as
/// one does when a termination signal ran its cleanup first, has shut down.
#[test]
fn an_unacknowledged_shutdown_with_a_successful_exit_succeeds() {
    // Stands in for the sidecar: announces readiness, consumes the 5-byte request, exits 0.
    let program = Path::new(env!("CARGO_TARGET_TMPDIR")).join("unacknowledging-reaper.sh");
    fs::write(
        &program,
        "#!/bin/sh\nprintf ORA1\nhead -c 5 >/dev/null\nexit 0\n",
    )
    .expect("write the stand-in reaper");
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755))
        .expect("make the stand-in reaper executable");

    ora_process::initialize_reaper(&program).expect("start the stand-in reaper");

    ora_process::shutdown_reaper().expect("shut the stand-in reaper down");
}
