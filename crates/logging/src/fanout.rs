use std::io::{self, Write};

use tracing::Metadata;
use tracing_subscriber::fmt::MakeWriter;

/// Composes two writer factories so a single formatting layer can emit identical bytes to both.
///
/// `stdout_and_file` mode used to stack two independent fmt layers, each with its own
/// `JsonEventFormatter`, so every event was serialized twice on the calling thread. The
/// fanout keeps one formatting layer and drives both sinks from one serialization pass,
/// halving the cost while preserving byte-for-byte output on both sinks.
#[derive(Clone, Debug)]
pub(crate) struct FanoutMakeWriter<A, B> {
    primary: A,
    secondary: B,
}

impl<A, B> FanoutMakeWriter<A, B> {
    /// Builds a fanout from an authoritative primary and a secondary sink.
    ///
    /// The primary is the user-visible sink whose error wins when both fail; the secondary
    /// is still written independently so its failures cannot drop the primary's bytes.
    pub(crate) fn new(primary: A, secondary: B) -> Self {
        Self { primary, secondary }
    }
}

impl<'writer, A, B> MakeWriter<'writer> for FanoutMakeWriter<A, B>
where
    A: MakeWriter<'writer>,
    B: MakeWriter<'writer>,
{
    type Writer = FanoutWriter<A::Writer, B::Writer>;

    /// Builds a writer that drives both sinks from one formatting pass.
    fn make_writer(&'writer self) -> Self::Writer {
        FanoutWriter {
            primary: self.primary.make_writer(),
            secondary: self.secondary.make_writer(),
        }
    }

    /// Forwards the event metadata to both sinks so per-event writer selection survives the fanout.
    ///
    /// The fmt layer always builds its writer through `make_writer_for`, and `MakeWriter`
    /// combinators implement their per-event routing there rather than in `make_writer`.
    /// The inherited default drops the metadata, which silently changes what a wrapped sink
    /// receives: `with_max_level`, for one, treats a metadata-less request as disabled and
    /// would discard every event on that sink.
    fn make_writer_for(&'writer self, meta: &Metadata<'_>) -> Self::Writer {
        FanoutWriter {
            primary: self.primary.make_writer_for(meta),
            secondary: self.secondary.make_writer_for(meta),
        }
    }
}

/// Forwards every byte to a primary and a secondary sink, keeping both writes independent.
#[derive(Debug)]
pub(crate) struct FanoutWriter<A, B> {
    primary: A,
    secondary: B,
}

impl<A, B> Write for FanoutWriter<A, B>
where
    A: Write,
    B: Write,
{
    /// Delegates to `write_all` so the trait-required `write` shares one sink-driving path
    /// and the same error semantics, instead of mirroring partial primary progress that no
    /// caller exercises. Returning the full buffer length is within the `Write` contract
    /// because `write_all` drives both sinks to completion before returning.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf).map(|()| buf.len())
    }

    /// Flushes both sinks, surfacing the primary's error first because it is observable.
    fn flush(&mut self) -> io::Result<()> {
        // Evaluate both flushes before short-circuiting so the secondary is never skipped.
        let primary_result = self.primary.flush();
        let secondary_result = self.secondary.flush();
        primary_result?;
        secondary_result
    }

    /// Writes to both sinks independently so one sink's failure cannot starve the other.
    ///
    /// The tracing fmt layer drives the writer through `write_all` with the complete
    /// serialized event, so overriding it here keeps the two-sink fanout as independent as
    /// the original two-layer topology: stdout trouble never drops a file event, and a file
    /// failure never suppresses the observable stdout stream. The primary's result wins when
    /// both fail because it is the user-visible sink.
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        let primary = self.primary.write_all(buf);
        let secondary = self.secondary.write_all(buf);
        primary?;
        secondary
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use pretty_assertions::assert_eq;
    use tracing::Level;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::fmt::layer;
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    use super::FanoutMakeWriter;
    use crate::test_support::with_recorded_trace_logging;

    /// Creates a shared in-memory sink so fanout tests can assert exact captured bytes.
    #[derive(Clone, Debug, Default)]
    struct CapturingSink {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturingSink {
        /// Returns the bytes captured so far so assertions compare full content, not counts.
        fn captured(&self) -> Vec<u8> {
            self.bytes.lock().unwrap().clone()
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturingSink {
        type Writer = CapturingWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturingWriter {
                bytes: self.bytes.clone(),
            }
        }
    }

    #[derive(Debug)]
    struct CapturingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A writer that always fails on write, used to verify the failing-sink error policy.
    #[derive(Debug, Default)]
    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("failing sink unavailable"))
        }

        fn write_all(&mut self, _buf: &[u8]) -> io::Result<()> {
            Err(io::Error::other("failing sink unavailable"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("failing flush unavailable"))
        }
    }

    impl<'writer> MakeWriter<'writer> for FailingWriter {
        type Writer = FailingWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            FailingWriter
        }
    }

    /// Verifies a single write fans identical bytes out to both sinks.
    #[test]
    fn mirrors_bytes_to_both_sinks() {
        let primary = CapturingSink::default();
        let secondary = CapturingSink::default();
        let mut writer = FanoutMakeWriter::new(primary.clone(), secondary.clone()).make_writer();

        writer.write_all(b"single formatting pass").unwrap();
        writer.flush().unwrap();

        assert_eq!(primary.captured(), b"single formatting pass");
        assert_eq!(secondary.captured(), b"single formatting pass");
    }

    /// Verifies multiple writes accumulate identically on both sinks.
    #[test]
    fn mirrors_chunked_writes_consistently() {
        let primary = CapturingSink::default();
        let secondary = CapturingSink::default();
        let mut writer = FanoutMakeWriter::new(primary.clone(), secondary.clone()).make_writer();

        writer.write_all(b"line one\n").unwrap();
        writer.write_all(b"line two\n").unwrap();
        writer.flush().unwrap();

        assert_eq!(primary.captured(), b"line one\nline two\n");
        assert_eq!(secondary.captured(), b"line one\nline two\n");
    }

    /// Verifies a failing secondary sink reports the error without dropping the primary bytes.
    #[test]
    fn surfaces_secondary_failures_without_dropping_primary_bytes() {
        let primary = CapturingSink::default();
        let mut writer = FanoutMakeWriter::new(primary.clone(), FailingWriter).make_writer();

        let result = writer.write_all(b"observable");

        assert!(result.is_err());
        assert_eq!(primary.captured(), b"observable");
    }

    /// Verifies a failing primary sink cannot starve the secondary of the event bytes.
    ///
    /// This is the regression guard for the fanout topology: the original two-layer design
    /// wrote each sink independently, so stdout trouble never dropped a file event. The
    /// fanout must preserve that independence by driving both `write_all` calls before
    /// short-circuiting on either error.
    #[test]
    fn writes_secondary_bytes_even_when_primary_fails() {
        let secondary = CapturingSink::default();
        let mut writer = FanoutMakeWriter::new(FailingWriter, secondary.clone()).make_writer();

        let result = writer.write_all(b"file survives stdout failure");

        assert!(result.is_err());
        assert_eq!(secondary.captured(), b"file survives stdout failure");
    }

    /// Verifies per-event writer selection on a wrapped sink survives the fanout.
    ///
    /// The fmt layer builds its writer through `make_writer_for`, and combinators such as
    /// `with_max_level` implement their filtering there rather than in `make_writer`. Without
    /// forwarding the metadata the wrapped sink resolves without level context and drops every
    /// event, so this test fails if the fanout ever inherits the default implementation.
    #[test]
    fn honors_per_event_writer_selection_on_each_sink() {
        let warn_only = CapturingSink::default();
        let unfiltered = CapturingSink::default();
        let fanout = FanoutMakeWriter::new(
            warn_only.clone().with_max_level(Level::WARN),
            unfiltered.clone(),
        );

        with_recorded_trace_logging(layer().with_writer(fanout).with_ansi(false), || {
            tracing::info!("below the filtered sink's level");
            tracing::warn!("above the filtered sink's level");
        });

        assert_eq!(captured_line_count(&warn_only), 1);
        assert_eq!(captured_line_count(&unfiltered), 2);
    }

    /// Counts the events a sink captured so level assertions do not depend on the event format.
    fn captured_line_count(sink: &CapturingSink) -> usize {
        String::from_utf8(sink.captured())
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
    }
}
