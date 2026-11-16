use super::{CheckoutHandOver, HandOverError, MAX_CONCURRENT_WALKS, WalkPermits};
use pretty_assertions::assert_eq;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

/// A hand-over with its own permits, so parallel tests never share the Node-wide ones.
fn isolated(sysctl: &Path) -> CheckoutHandOver {
    CheckoutHandOver {
        protected_hardlinks: sysctl.to_path_buf(),
        permits: Arc::new(WalkPermits::new(MAX_CONCURRENT_WALKS)),
    }
}

/// However many starts and deliveries ask at once, at most nine walks overlap, and every one
/// of them still runs once a permit is free.
#[test]
fn at_most_nine_walks_run_at_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sysctl = temp.path().join("protected_hardlinks");
    std::fs::write(&sysctl, "1\n").expect("sysctl stand-in");
    let hand_over = isolated(&sysctl);
    let (active, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let callers = 4 * MAX_CONCURRENT_WALKS;
    let start = Arc::new(Barrier::new(callers));

    let results: Vec<_> = (0..callers)
        .map(|_| {
            let (hand_over, active, peak, start) = (
                hand_over.clone(),
                active.clone(),
                peak.clone(),
                start.clone(),
            );
            thread::spawn(move || {
                start.wait();
                hand_over
                    .walk(|| {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(/*millis*/ 20));
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(1)
                    })
                    .map_err(|error| error.to_string())
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|walker| walker.join().expect("walker"))
        .collect();

    assert_eq!(
        (results, peak.load(Ordering::SeqCst)),
        // Nine, as the documented descriptor budget assumes.
        (vec![Ok(1); callers], 9)
    );
}

/// The setting is read again before every walk: a host flipped away from `1` after the Node
/// started refuses the next walk before it touches the checkout, and the walk runs again once
/// the protection is back.
#[test]
fn every_walk_rereads_protected_hardlinks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sysctl = temp.path().join("protected_hardlinks");
    let hand_over = isolated(&sysctl);
    let walked = AtomicBool::new(false);
    let mut outcomes = Vec::new();
    for value in [Some("1\n"), Some("0\n"), None, Some("1\n")] {
        match value {
            Some(value) => std::fs::write(&sysctl, value).expect("sysctl stand-in"),
            None => std::fs::remove_file(&sysctl).expect("no sysctl"),
        }
        walked.store(false, Ordering::SeqCst);
        let result = hand_over.walk(|| {
            walked.store(true, Ordering::SeqCst);
            Ok(0)
        });
        outcomes.push((
            match result {
                Ok(changed) => format!("walked {changed}"),
                Err(HandOverError::HardlinksUnprotected(error)) => error.to_string(),
                Err(error) => format!("unexpected: {error}"),
            },
            walked.load(Ordering::SeqCst),
        ));
    }
    let named = |found: &str| {
        format!(
            "process.workload_uid requires the sysctl fs.protected_hardlinks = 1 ({}), but {found}",
            sysctl.display()
        )
    };
    let missing = outcomes[2].0.clone();
    assert!(
        missing.starts_with(&named("it could not be read")),
        "{missing}"
    );
    assert_eq!(
        outcomes,
        vec![
            ("walked 0".to_owned(), true),
            (named("it reads \"0\""), false),
            (missing, false),
            ("walked 0".to_owned(), true),
        ]
    );
}
