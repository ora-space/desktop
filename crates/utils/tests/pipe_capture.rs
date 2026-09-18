#![cfg(unix)]

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use ora_utils::process::{PipeCapture, PipeRead, PipeReadState};
use pretty_assertions::assert_eq;

/// Waits for a concrete reader fact, with a bounded failure deadline.
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while !ready() {
        assert!(Instant::now() < deadline, "reader timed out");
        std::thread::sleep(Duration::from_millis(/*millis*/ 1));
    }
}

/// Exact capacity is complete; the first excess byte makes truncation sticky through EOF.
#[test]
fn bounded_prefix_and_offsets() -> io::Result<()> {
    for (input, limit, truncated) in [
        (b"abcd".as_slice(), 4, false),
        (b"abcde", 4, true),
        (b"a", 0, true),
        (b"", 0, false),
    ] {
        let (reader, mut writer) = UnixStream::pair()?;
        let capture = PipeCapture::start(File::from(OwnedFd::from(reader)), limit)?;
        writer.write_all(input)?;
        drop(writer);
        until(|| {
            capture
                .read(/*offset*/ 0, /*max_bytes*/ 0)
                .is_ok_and(|read| read.state == PipeReadState::Eof)
        });
        let retained = input.len().min(limit);
        assert_eq!(
            capture.read(/*offset*/ 0, /*max_bytes*/ 20)?,
            PipeRead {
                bytes: input[..retained].to_vec(),
                retained,
                truncated,
                state: PipeReadState::Eof,
            }
        );
        assert_eq!(capture.failed(), truncated);
        assert_eq!(
            capture.read(retained, /*max_bytes*/ 1)?.bytes,
            Vec::<u8>::new()
        );
        assert!(
            matches!(capture.read(retained + 1, /*max_bytes*/ 1), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
        if retained > 1 {
            assert_eq!(capture.read(/*offset*/ 1, /*max_bytes*/ 2)?.bytes, b"bc");
        }
    }
    Ok(())
}

/// Cancellation closes an otherwise indefinitely open reader without waiting for the writer.
#[test]
fn drop_releases_open_pipe() -> io::Result<()> {
    let (reader, mut writer) = UnixStream::pair()?;
    let capture = PipeCapture::start(File::from(OwnedFd::from(reader)), /*limit*/ 0)?;
    writer.set_nonblocking(/*nonblocking*/ true)?;
    drop(capture);
    until(
        || matches!(writer.write(b"x"), Err(error) if matches!(error.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset)),
    );
    Ok(())
}
