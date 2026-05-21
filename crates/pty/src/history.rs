use crate::types::{PtyOutputChunk, PtySessionReplay};
use std::collections::VecDeque;

/// Keeps ordered PTY output bounded so reconnect replay cannot grow without limit.
#[derive(Debug)]
pub struct OutputHistory {
    max_bytes: usize,
    total_bytes: usize,
    chunks: VecDeque<PtyOutputChunk>,
}

impl OutputHistory {
    /// Builds a bounded PTY output history using the provided byte cap.
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            total_bytes: 0,
            chunks: VecDeque::new(),
        }
    }

    /// Appends one output chunk and trims the oldest buffered output when the cap is exceeded.
    pub fn push(&mut self, data: String) {
        let chunk = PtyOutputChunk { data };
        self.total_bytes += chunk.data.len();
        self.chunks.push_back(chunk);

        while self.total_bytes > self.max_bytes {
            if let Some(removed_chunk) = self.chunks.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(removed_chunk.data.len());
            } else {
                break;
            }
        }
    }

    /// Returns the current replay snapshot in append order.
    pub fn replay(&self) -> PtySessionReplay {
        PtySessionReplay {
            chunks: self.chunks.iter().cloned().collect(),
        }
    }

    /// Clears every buffered chunk after the PTY exits.
    pub fn clear(&mut self) {
        self.total_bytes = 0;
        self.chunks.clear();
    }
}
