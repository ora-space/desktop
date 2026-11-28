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

/// The descriptor budget a Node document states: the formula the minimum is derived by, then
/// the `RLIMIT_NOFILE` minimum and recommendation that follow it.
#[derive(Debug, Eq, PartialEq)]
struct DocumentedBudget {
    walks: usize,
    per_walk: usize,
    baseline: usize,
    rounded: usize,
    minimum: usize,
    recommended: usize,
}

/// The number a text ends with, without its thousands separators.
fn trailing_number(text: &str) -> usize {
    let digits: String = text
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit() || *character == ',')
        .collect();
    digits
        .chars()
        .rev()
        .collect::<String>()
        .replace(',', "")
        .parse()
        .expect("a number with thousands separators")
}

/// The number a text starts with, without its thousands separators, and the text after it.
fn leading_number(text: &str) -> (usize, &str) {
    let end = text
        .find(|character: char| !(character.is_ascii_digit() || character == ','))
        .unwrap_or(text.len());
    (
        text[..end]
            .replace(',', "")
            .parse()
            .expect("a number with thousands separators"),
        &text[end..],
    )
}

/// Reads the budget out of a document's prose. Both language versions state it with the same
/// numerals: `walks × per-walk + baseline ≈ rounded`, then the minimum after "at least" or
/// "至少", and the recommendation before "recommended" or after "推荐".
fn documented_budget(text: &str) -> DocumentedBudget {
    // Line wrapping must not split the formula, so every run of whitespace counts as one space.
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let (before, rest) = text
        .split_once(" × ")
        .expect("the document states the per-walk formula");
    let (per_walk, rest) = leading_number(rest);
    let rest = rest.strip_prefix(" + ").expect("the formula's baseline");
    let (baseline, rest) = leading_number(rest);
    let rest = rest
        .strip_prefix(" ≈ ")
        .expect("the formula's rounded total");
    let (rounded, _) = leading_number(rest);
    let (_, after_minimum) = text
        .split_once("at least ")
        .or_else(|| text.split_once("至少 "))
        .expect("the document states the minimum");
    let (minimum, _) = leading_number(after_minimum);
    let recommended = match (text.split_once(" recommended"), text.split_once("推荐 ")) {
        (Some((before, _)), _) => trailing_number(before),
        (None, Some((_, after))) => leading_number(after).0,
        (None, None) => panic!("the document states the recommendation"),
    };
    DocumentedBudget {
        walks: trailing_number(before),
        per_walk,
        baseline,
        rounded,
        minimum,
        recommended,
    }
}

/// The documented descriptor minimum is derived from the two bounds this module and the walk
/// own, so it has to stay consistent with them: both Node documents state the formula with the
/// constants in force, and their `RLIMIT_NOFILE` minimum covers what it adds up to. No test can
/// prove a deployment provides the limit, since the Node neither reads nor raises it; a bound
/// changed without the documents, or a minimum lowered below the derivation, fails here.
#[test]
fn the_documented_descriptor_minimum_follows_the_walk_bounds() {
    let documents = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/node");
    for document in ["agent-sessions.md", "agent-sessions.zh.md"] {
        let text = std::fs::read_to_string(documents.join(document)).expect("Node document");
        let budget = documented_budget(&text);
        assert_eq!(
            (budget.walks, budget.per_walk),
            (MAX_CONCURRENT_WALKS, ora_utils::fs::MAX_OWN_TREE_DEPTH + 3),
            "{document}: the formula must name the bounds in force"
        );
        // Each figure is at least the one before it, so the minimum asked for covers the need.
        assert!(
            budget.rounded >= budget.walks * budget.per_walk + budget.baseline,
            "{document}: {budget:?}"
        );
        assert!(budget.minimum >= budget.rounded, "{document}: {budget:?}");
        assert!(
            budget.recommended >= budget.minimum,
            "{document}: {budget:?}"
        );
    }
}
