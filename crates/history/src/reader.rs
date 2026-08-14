use crate::error::HistoryError;
use crate::path::history_path;
use crate::record::HistoryLine;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::Path;

/// Describes whether every durable record could be restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryIntegrity {
    /// Every complete line was decoded successfully.
    Complete,
    /// Complete lines were unreadable and their timeline positions are unknown.
    Damaged { unreadable_lines: NonZeroUsize },
}

/// One session's complete history, restored to conversation order.
pub struct SessionHistory {
    /// Every surviving line, ordered by position with duplicates resolved.
    pub lines: Vec<HistoryLine>,
    /// The position the next appended record should claim.
    pub next_seq: u32,
    /// Whether complete lines had to be omitted while reading the record.
    ///
    /// A torn last line is normal after a crash and is not counted here. Anything
    /// else means the file lost content that no longer has a place in the
    /// timeline, which callers are expected to surface rather than absorb.
    pub integrity: HistoryIntegrity,
}

impl SessionHistory {
    /// Reports whether this session has no history to replay.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Reads one session's history, tolerating a write interrupted mid-line.
///
/// A missing file is an empty history rather than an error: a session that has
/// never been prompted has nothing recorded, and neither does one created before
/// Ora owned its own history.
///
/// The file is read whole. Every caller — replay, handoff rendering, and resuming
/// the position counter — needs all of it, so a reverse scan would buy nothing.
pub fn read_session_history(root: &Path, session_id: &str) -> Result<SessionHistory, HistoryError> {
    let path = history_path(root, session_id)?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SessionHistory {
                lines: Vec::new(),
                next_seq: 0,
                integrity: HistoryIntegrity::Complete,
            });
        }
        Err(source) => return Err(HistoryError::Read { path, source }),
    };

    // Split on bytes rather than decoding first: a write cut short can leave a
    // partial UTF-8 sequence at the tail, which would otherwise fail the whole
    // file instead of only its unfinished last line.
    let ends_with_newline = bytes.last() == Some(&b'\n');
    let segments: Vec<&[u8]> = bytes
        .split(|byte| *byte == b'\n')
        .filter(|segment| !segment.is_empty())
        .collect();

    let mut parsed = Vec::with_capacity(segments.len());
    let mut dropped_lines = 0;
    for (index, segment) in segments.iter().enumerate() {
        match serde_json::from_slice::<HistoryLine>(segment) {
            Ok(line) => parsed.push(line),
            Err(_) => {
                // An unterminated final segment is the signature of a write the
                // process never finished; anything else lost real content.
                let torn_tail = !ends_with_newline && index + 1 == segments.len();
                if !torn_tail {
                    dropped_lines += 1;
                }
            }
        }
    }

    let next_seq = parsed
        .iter()
        .map(|line| line.seq)
        .max()
        .map_or(0, |seq| seq.saturating_add(1));
    let integrity = match NonZeroUsize::new(dropped_lines) {
        Some(unreadable_lines) => HistoryIntegrity::Damaged { unreadable_lines },
        None => HistoryIntegrity::Complete,
    };
    Ok(SessionHistory {
        lines: order_lines(parsed),
        next_seq,
        integrity,
    })
}

/// Restores conversation order and resolves positions written more than once.
///
/// Records are appended when they settle, not when they appeared, so file order
/// is not conversation order. A tool call that changed after being written is
/// appended again under its original position, and the last of those is the one
/// the conversation ended with.
fn order_lines(lines: Vec<HistoryLine>) -> Vec<HistoryLine> {
    let mut seen = HashSet::with_capacity(lines.len());
    // Scanning backwards keeps the last record written for a position and drops
    // the ones it corrected. What survives is unique per position, so the sort
    // that follows fully determines the order and the reversed scan order here
    // does not need undoing.
    let mut latest: Vec<HistoryLine> = lines
        .into_iter()
        .rev()
        .filter(|line| seen.insert(line.seq))
        .collect();
    latest.sort_by_key(|line| line.seq);
    latest
}
