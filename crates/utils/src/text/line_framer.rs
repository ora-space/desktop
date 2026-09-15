//! Splits an incrementally arriving byte stream into newline-delimited logical records while
//! keeping memory bounded.
//!
//! The framer never trusts the producer to emit a newline: once a record grows past the
//! configured limit it is flushed as an ordered sequence of fragments instead of being held
//! until a newline finally arrives. A consumer can therefore treat a `Line` as one complete
//! record and reassemble a fragmented one from its sequence number and index without ever
//! buffering more than the limit itself.

/// One record produced by [`BoundedLineFramer`]; never includes the terminating newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineFrame {
    /// A whole newline-terminated record (or the trailing record at end of input) within limit.
    Line(Vec<u8>),
    /// One piece of a record that exceeded the limit before its newline arrived.
    ///
    /// Pieces of one oversized record share a `sequence`; `index` orders them and `last` marks
    /// the piece that carried the newline or the end of input.
    Fragment {
        sequence: u64,
        index: u32,
        last: bool,
        bytes: Vec<u8>,
    },
}

/// Incremental, bounded newline framer.
#[derive(Debug)]
pub struct BoundedLineFramer {
    max_record_bytes: usize,
    buffer: Vec<u8>,
    /// `Some` while the current record has already been partially flushed as fragments.
    overflow: Option<Overflow>,
    next_sequence: u64,
}

#[derive(Debug)]
struct Overflow {
    sequence: u64,
    next_index: u32,
}

impl BoundedLineFramer {
    /// Creates a framer that flushes any record reaching `max_record_bytes` as fragments.
    ///
    /// # Panics
    ///
    /// Panics when `max_record_bytes` is zero, which could never hold a record.
    pub fn new(max_record_bytes: usize) -> Self {
        assert!(max_record_bytes > 0, "record limit must be positive");
        Self {
            max_record_bytes,
            buffer: Vec::new(),
            overflow: None,
            next_sequence: 0,
        }
    }

    /// Feeds one chunk of bytes and returns every frame it completes, in order.
    ///
    /// Chunk boundaries carry no meaning: the same byte sequence produces the same frames however
    /// it is split, which is what makes the framer safe on top of pipe reads.
    pub fn push(&mut self, mut chunk: &[u8]) -> Vec<LineFrame> {
        let mut frames = Vec::new();
        while !chunk.is_empty() {
            match chunk.iter().position(|byte| *byte == b'\n') {
                Some(newline) => {
                    let (line, rest) = chunk.split_at(newline);
                    self.append_bounded(line, &mut frames);
                    frames.push(self.take_record(/*last*/ true));
                    chunk = &rest[1..];
                }
                None => {
                    self.append_bounded(chunk, &mut frames);
                    chunk = &[];
                }
            }
        }
        frames
    }

    /// Flushes whatever precedes end of input as a final frame, if anything was buffered.
    pub fn finish(&mut self) -> Option<LineFrame> {
        if self.buffer.is_empty() && self.overflow.is_none() {
            return None;
        }
        Some(self.take_record(/*last*/ true))
    }

    /// Appends bytes to the current record, emitting fragments whenever the limit is reached.
    ///
    /// A record that exactly fills the buffer is only flushed once more input proves it is not
    /// complete; otherwise a record of exactly the limit would be split for no reason.
    fn append_bounded(&mut self, mut bytes: &[u8], frames: &mut Vec<LineFrame>) {
        while !bytes.is_empty() {
            let room = self.max_record_bytes - self.buffer.len();
            let take = room.min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.buffer.len() == self.max_record_bytes && !bytes.is_empty() {
                frames.push(self.take_record(/*last*/ false));
            }
        }
    }

    /// Drains the buffer into a `Line` or the next fragment of an oversized record.
    fn take_record(&mut self, last: bool) -> LineFrame {
        let bytes = std::mem::take(&mut self.buffer);
        let mut overflow = self.overflow.take();
        if overflow.is_none() && last {
            return LineFrame::Line(bytes);
        }
        let state = overflow.get_or_insert_with(|| {
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            Overflow {
                sequence,
                next_index: 0,
            }
        });
        let frame = LineFrame::Fragment {
            sequence: state.sequence,
            index: state.next_index,
            last,
            bytes,
        };
        state.next_index += 1;
        if !last {
            self.overflow = overflow;
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundedLineFramer, LineFrame};
    use pretty_assertions::assert_eq;

    /// Collects every frame from one framer fed `chunks` and then finished.
    fn frames_for(limit: usize, chunks: &[&[u8]]) -> Vec<LineFrame> {
        let mut framer = BoundedLineFramer::new(limit);
        let mut frames = Vec::new();
        for chunk in chunks {
            frames.extend(framer.push(chunk));
        }
        frames.extend(framer.finish());
        frames
    }

    /// Splitting the input at every possible boundary produces identical frames.
    #[test]
    fn frames_do_not_depend_on_chunk_boundaries() {
        let input = b"alpha\nbeta gamma\n\ndelta";
        let whole = frames_for(8, &[input]);
        for split in 0..=input.len() {
            let (head, tail) = input.split_at(split);
            assert_eq!(frames_for(8, &[head, tail]), whole, "split at {split}");
        }
        assert_eq!(
            whole,
            vec![
                LineFrame::Line(b"alpha".to_vec()),
                LineFrame::Fragment {
                    sequence: 0,
                    index: 0,
                    last: false,
                    bytes: b"beta gam".to_vec(),
                },
                LineFrame::Fragment {
                    sequence: 0,
                    index: 1,
                    last: true,
                    bytes: b"ma".to_vec(),
                },
                LineFrame::Line(Vec::new()),
                LineFrame::Line(b"delta".to_vec()),
            ]
        );
    }

    /// A record of exactly the limit is one line, not a fragment plus an empty tail.
    #[test]
    fn a_record_exactly_at_the_limit_stays_whole() {
        assert_eq!(
            frames_for(4, &[b"abcd\nef"]),
            vec![
                LineFrame::Line(b"abcd".to_vec()),
                LineFrame::Line(b"ef".to_vec())
            ]
        );
    }

    /// An unterminated oversized record at end of input still ends with a `last` fragment, and
    /// separate oversized records get distinct sequence numbers.
    #[test]
    fn oversized_records_are_sequenced_and_closed_at_end_of_input() {
        assert_eq!(
            frames_for(3, &[b"abcdefg\nhijklm"]),
            vec![
                LineFrame::Fragment {
                    sequence: 0,
                    index: 0,
                    last: false,
                    bytes: b"abc".to_vec(),
                },
                LineFrame::Fragment {
                    sequence: 0,
                    index: 1,
                    last: false,
                    bytes: b"def".to_vec(),
                },
                LineFrame::Fragment {
                    sequence: 0,
                    index: 2,
                    last: true,
                    bytes: b"g".to_vec(),
                },
                LineFrame::Fragment {
                    sequence: 1,
                    index: 0,
                    last: false,
                    bytes: b"hij".to_vec(),
                },
                LineFrame::Fragment {
                    sequence: 1,
                    index: 1,
                    last: true,
                    bytes: b"klm".to_vec(),
                },
            ]
        );
    }

    /// Finishing an empty framer yields nothing rather than an empty line.
    #[test]
    fn finish_without_buffered_bytes_yields_nothing() {
        let mut framer = BoundedLineFramer::new(8);
        assert_eq!(framer.push(b"x\n"), vec![LineFrame::Line(b"x".to_vec())]);
        assert_eq!(framer.finish(), None);
    }
}
