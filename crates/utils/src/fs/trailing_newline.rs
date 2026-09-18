use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// How a line-oriented file ends, as far as appending another line is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTail {
    /// The file holds no bytes; appending starts a fresh first line.
    Empty,
    /// The last byte is `\n`; appending starts a fresh line.
    Terminated,
    /// The last byte is not `\n`: a previous writer stopped mid-line, so appending would join
    /// the new line onto the broken one.
    Unterminated,
}

/// Classifies the tail of `file` by reading only its final byte.
///
/// A crash or partial write leaves a line-oriented file without its closing newline; an
/// appender that does not check would glue its first record onto that fragment and corrupt
/// both. The check is deliberately bounded to one seek and one byte so it stays cheap on a
/// file of any size. The cursor position is left at the end of the file.
pub fn classify_line_tail(file: &mut File) -> io::Result<LineTail> {
    let length = file.metadata()?.len();
    if length == 0 {
        return Ok(LineTail::Empty);
    }
    file.seek(SeekFrom::Start(length - 1))?;
    let mut last = [0_u8; 1];
    file.read_exact(&mut last)?;
    Ok(if last[0] == b'\n' {
        LineTail::Terminated
    } else {
        LineTail::Unterminated
    })
}

#[cfg(test)]
mod tests {
    use super::{LineTail, classify_line_tail};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    /// Each tail shape is recognized from its last byte alone.
    #[test]
    fn classifies_empty_terminated_and_unterminated_files() {
        let temp = TempDir::new().expect("temp dir");
        let cases: [(&str, &[u8], LineTail); 3] = [
            ("empty.log", b"", LineTail::Empty),
            (
                "terminated.log",
                b"{\"a\":1}\n{\"b\":2}\n",
                LineTail::Terminated,
            ),
            (
                "unterminated.log",
                b"{\"a\":1}\n{\"b\":",
                LineTail::Unterminated,
            ),
        ];
        let observed = cases
            .iter()
            .map(|(name, content, _)| {
                let path = temp.path().join(name);
                std::fs::write(&path, content).expect("write");
                let mut file = std::fs::File::open(&path).expect("open");
                classify_line_tail(&mut file).expect("classify")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed,
            cases.iter().map(|(_, _, tail)| *tail).collect::<Vec<_>>()
        );
    }
}
