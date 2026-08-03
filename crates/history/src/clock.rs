use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Supplies the wall-clock instant stamped onto every appended history line.
///
/// History files are meant to be read by a person with `jq` or a text editor, so
/// the timestamps are local rather than UTC. Implementations are expected to
/// return a time carrying the local UTC offset; tests supply a fixed instant so
/// assembled output stays byte-comparable.
pub trait HistoryClock {
    /// Returns the current local time used for the next appended line.
    fn now_local(&self) -> OffsetDateTime;
}

/// Formats one instant as the RFC 3339 text stored in a line's `at` field.
///
/// Falls back to the Unix epoch only when the platform hands back a time the
/// format cannot represent. Losing the timestamp is preferable to dropping the
/// record, because the record is the conversation and the timestamp is not.
pub(crate) fn format_timestamp(at: OffsetDateTime) -> String {
    at.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Reports a caller-supplied instant, so assembled files stay deterministic.
#[derive(Clone, Copy, Debug)]
pub struct FixedHistoryClock(OffsetDateTime);

impl FixedHistoryClock {
    /// Pins every timestamp this clock reports to one instant.
    pub fn new(at: OffsetDateTime) -> Self {
        Self(at)
    }
}

impl HistoryClock for FixedHistoryClock {
    fn now_local(&self) -> OffsetDateTime {
        self.0
    }
}
