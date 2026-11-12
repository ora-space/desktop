#![cfg(unix)]
//! Whether a host can tell that its reaper is gone. This file is its own test binary because the
//! reaper client is process-global and can be started only once.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

/// A reaper that was never started is not one that exited, a running one is not either, and one
/// that ended without being asked to is reported from then on.
#[test]
fn a_reaper_that_exits_on_its_own_is_reported() {
    // Stands in for the sidecar: announces readiness, then stays until the stop file appears.
    let program = Path::new(env!("CARGO_TARGET_TMPDIR")).join("exiting-reaper.sh");
    let stop = program.with_extension("stop");
    let _ = fs::remove_file(&stop);
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nprintf ORA1\nwhile [ ! -e '{}' ]; do sleep 0.05; done\n",
            stop.display()
        ),
    )
    .expect("write the stand-in reaper");
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755))
        .expect("make the stand-in reaper executable");

    let before_start = ora_process::reaper_has_exited();
    ora_process::initialize_reaper(&program).expect("start the stand-in reaper");
    let while_running = ora_process::reaper_has_exited();
    fs::write(&stop, "").expect("let the stand-in reaper end");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ora_process::reaper_has_exited() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        (
            before_start,
            while_running,
            ora_process::reaper_has_exited()
        ),
        (false, false, true)
    );
}
