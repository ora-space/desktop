//! Reading a prior bundle's header before Git sees it (Node restore ADR D3 step 1).
//!
//! `git bundle verify` fails the same way for a missing base commit and for a broken bundle, but
//! Cloud treats the two differently, so the Node reads the prerequisites itself first. Only the
//! shapes a Revision delivery creates are accepted: a v2 or v3 bundle without filters, whose only
//! head is one Revision ref.
use ora_node_protocol::{CommitId, REVISION_REF_PREFIX};
use std::io::{self, BufRead, Read};
use std::path::Path;

/// The header is read up to this many bytes; a Revision bundle's is far smaller, and a larger one
/// is not a bundle the Node would accept anyway.
const HEADER_LIMIT: u64 = 1024 * 1024;

/// What a Revision bundle needs and holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BundleHeader {
    /// Commits the bundle is relative to, which the checkout must contain.
    pub(crate) prerequisites: Vec<CommitId>,
    /// The single Revision ref.
    pub(crate) head: String,
    pub(crate) final_commit: CommitId,
}

/// Why a header was refused.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HeaderError {
    #[error("bundle header could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("not a v2 or v3 Git bundle")]
    Signature,
    #[error("bundle uses an unsupported capability")]
    Capability,
    #[error("malformed bundle header line")]
    Line,
    #[error("bundle header is not terminated within the size limit")]
    Unterminated,
    #[error("bundle must hold exactly one Revision ref")]
    Heads,
}

/// Reads and parses the header of the bundle at `path`.
pub(crate) fn read_header(path: &Path) -> Result<BundleHeader, HeaderError> {
    let mut reader = io::BufReader::new(std::fs::File::open(path)?.take(HEADER_LIMIT));
    let mut lines = Vec::new();
    loop {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line)? == 0 || line.last() != Some(&b'\n') {
            return Err(HeaderError::Unterminated);
        }
        line.pop();
        if line.is_empty() {
            break;
        }
        lines.push(String::from_utf8(line).map_err(|_| HeaderError::Line)?);
    }
    parse_header(&lines)
}

/// Parses the header lines before the blank line that starts the pack.
fn parse_header(lines: &[String]) -> Result<BundleHeader, HeaderError> {
    let (signature, rest) = lines.split_first().ok_or(HeaderError::Signature)?;
    let v3 = match signature.as_str() {
        "# v2 git bundle" => false,
        "# v3 git bundle" => true,
        _ => return Err(HeaderError::Signature),
    };
    let mut prerequisites = Vec::new();
    let mut heads = Vec::new();
    for line in rest {
        if let Some(capability) = line.strip_prefix('@') {
            // Only the object format is harmless; a filter means a partial bundle that could not
            // restore the full tree.
            if !v3 || !matches!(capability, "object-format=sha1" | "object-format=sha256") {
                return Err(HeaderError::Capability);
            }
        } else if let Some(prerequisite) = line.strip_prefix('-') {
            let oid = prerequisite.split(' ').next().unwrap_or_default();
            prerequisites.push(object_id(oid)?);
        } else {
            let (oid, name) = line.split_once(' ').ok_or(HeaderError::Line)?;
            heads.push((object_id(oid)?, name.to_owned()));
        }
    }
    let [(final_commit, head)] = <[_; 1]>::try_from(heads).map_err(|_| HeaderError::Heads)?;
    let valid_ref = head
        .strip_prefix(REVISION_REF_PREFIX)
        .is_some_and(|suffix| ora_utils::GitBranchName::parse(suffix).is_ok());
    if !valid_ref {
        return Err(HeaderError::Heads);
    }
    Ok(BundleHeader {
        prerequisites,
        head,
        final_commit,
    })
}

/// Requires a full lowercase hexadecimal SHA-1 or SHA-256 object ID, as Git writes them.
fn object_id(value: &str) -> Result<CommitId, HeaderError> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Ok(CommitId::new(value));
    }
    Err(HeaderError::Line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const BASE: &str = "0123456789abcdef0123456789abcdef01234567";
    const FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";

    /// Header lines as owned strings.
    fn lines(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    /// Both bundle versions a Revision delivery can write parse into prerequisites and the head.
    #[test]
    fn parses_v2_and_v3_revision_bundles() {
        let expected = BundleHeader {
            prerequisites: vec![CommitId::new(BASE)],
            head: "refs/ora/revisions/run-1".into(),
            final_commit: CommitId::new(FINAL),
        };
        for header in [
            lines(&[
                "# v2 git bundle",
                &format!("-{BASE} base subject"),
                &format!("{FINAL} refs/ora/revisions/run-1"),
            ]),
            lines(&[
                "# v3 git bundle",
                "@object-format=sha1",
                &format!("-{BASE}"),
                &format!("{FINAL} refs/ora/revisions/run-1"),
            ]),
        ] {
            assert_eq!(parse_header(&header).unwrap(), expected);
        }
    }

    /// Anything but one Revision head, a filter, or an unknown format is refused.
    #[test]
    fn refuses_other_shapes() {
        let cases = [
            lines(&[
                "# v4 git bundle",
                &format!("{FINAL} refs/ora/revisions/run-1"),
            ]),
            lines(&[
                "# v3 git bundle",
                "@filter=blob:none",
                &format!("{FINAL} refs/ora/revisions/run-1"),
            ]),
            lines(&[
                "# v2 git bundle",
                "@object-format=sha1",
                &format!("{FINAL} refs/ora/revisions/run-1"),
            ]),
            lines(&["# v2 git bundle", &format!("{FINAL} refs/heads/main")]),
            lines(&[
                "# v2 git bundle",
                &format!("{FINAL} refs/ora/revisions/run-1"),
                &format!("{BASE} refs/ora/revisions/run-2"),
            ]),
            lines(&["# v2 git bundle", &format!("-{BASE}")]),
            lines(&[
                "# v2 git bundle",
                "-0123 short",
                &format!("{FINAL} refs/ora/revisions/run-1"),
            ]),
            lines(&[
                "# v2 git bundle",
                &format!("{FINAL} refs/ora/revisions/../heads/main"),
            ]),
        ];
        for header in cases {
            assert!(parse_header(&header).is_err(), "{header:?}");
        }
    }
}
