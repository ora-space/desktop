//! What a Git Server answered, read from Git's output (Node delivery push ADR D4, D6).
//!
//! Git reports HTTP statuses and refused refs only in its human-readable output, so the
//! classification depends on its wording. Anything unrecognized counts as retryable: a wrong
//! guess costs at most two more attempts, while calling a passing failure permanent would fail
//! work that could have succeeded.
use gitlancer::GitExecError;

/// Why a Git Server request did not succeed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GitServerFailure {
    /// `401`, `403` or another `4xx` other than `429`: Cloud's current state refuses the request,
    /// and repeating it within the same attempt cannot change that.
    Denied,
    /// The server has no ref matching what was asked for.
    MissingRef,
    /// `5xx`, `429`, connection, DNS, TLS, low-speed abort, deadline or anything unrecognized,
    /// after the retries ran out.
    Unavailable,
    /// The Node began stopping; the work resumes or ends by its own rules.
    Stopped,
}

/// The fate of the pushed ref according to `git push --porcelain`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PushReport {
    /// The ref was created or moved.
    Updated,
    /// The ref already pointed at the pushed commit.
    UpToDate,
    /// The declared old value did not match the server's.
    Stale,
    /// The server or Git declined the update for another reason.
    Rejected,
}

/// Classifies a failed transport-level command from Git's diagnostics.
pub(crate) fn classify(error: &GitExecError) -> GitServerFailure {
    let GitExecError::NonZeroExit { stderr, .. } = error else {
        return GitServerFailure::Unavailable;
    };
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("couldn't find remote ref") || lower.contains("no such remote ref") {
        return GitServerFailure::MissingRef;
    }
    let status = lower
        .split("returned error: ")
        .nth(1)
        .and_then(|rest| rest.get(..3))
        .and_then(|code| code.parse::<u16>().ok());
    match status {
        Some(429) => GitServerFailure::Unavailable,
        Some(400..=499) => GitServerFailure::Denied,
        Some(_) | None => GitServerFailure::Unavailable,
    }
}

/// Reads the porcelain line of `destination`; `None` when Git printed none, which means the push
/// never reached the ref update (a transport failure).
pub(crate) fn push_report(stdout: &str, destination: &str) -> Option<PushReport> {
    stdout.lines().find_map(|line| {
        let mut fields = line.split('\t');
        let flag = fields.next()?;
        let (_, to) = fields.next()?.rsplit_once(':')?;
        if to != destination {
            return None;
        }
        let summary = fields.next().unwrap_or_default();
        Some(match flag {
            "=" => PushReport::UpToDate,
            "!" if summary.contains("stale info") => PushReport::Stale,
            "!" => PushReport::Rejected,
            _ => PushReport::Updated,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Builds a failed command with the given diagnostics.
    fn failed(stderr: &str) -> GitExecError {
        GitExecError::NonZeroExit {
            code: Some(128),
            args: Vec::new(),
            stdout: String::new(),
            stderr: stderr.to_owned(),
        }
    }

    /// Statuses map to their ADR categories; unknown wording stays retryable.
    #[test]
    fn failures_follow_the_status_and_ref_answers() {
        let cases = [
            "fatal: unable to access 'https://h/r.git/': The requested URL returned error: 403",
            "fatal: unable to access 'https://h/r.git/': The requested URL returned error: 401",
            "fatal: unable to access 'https://h/r.git/': The requested URL returned error: 503",
            "error: RPC failed; HTTP 429 curl 22 The requested URL returned error: 429",
            "fatal: couldn't find remote ref refs/ora/deliveries/d",
            "fatal: unable to access 'https://h/r.git/': Could not resolve host: h",
            "something new",
        ];
        assert_eq!(
            cases.map(|stderr| classify(&failed(stderr))),
            [
                GitServerFailure::Denied,
                GitServerFailure::Denied,
                GitServerFailure::Unavailable,
                GitServerFailure::Unavailable,
                GitServerFailure::MissingRef,
                GitServerFailure::Unavailable,
                GitServerFailure::Unavailable,
            ]
        );
    }

    /// The destination's porcelain line decides the push report.
    #[test]
    fn porcelain_lines_name_the_fate_of_the_destination() {
        let report =
            |line: &str| push_report(&format!("To https://h/r.git\n{line}\nDone\n"), "refs/x");
        assert_eq!(
            [
                report("*\tabc:refs/x\t[new reference]"),
                report(" \tabc:refs/x\t1..2"),
                report("=\tabc:refs/x\t[up to date]"),
                report("!\tabc:refs/x\t[rejected] (stale info)"),
                report("!\tabc:refs/x\t[remote rejected] (pre-receive hook declined)"),
                report("*\tabc:refs/y\t[new reference]"),
            ],
            [
                Some(PushReport::Updated),
                Some(PushReport::Updated),
                Some(PushReport::UpToDate),
                Some(PushReport::Stale),
                Some(PushReport::Rejected),
                None,
            ]
        );
    }
}
