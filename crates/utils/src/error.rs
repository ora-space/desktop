//! Generic error rendering.

use std::error::Error;

/// Renders an error followed by all of its causes on one line.
///
/// `Display` alone stops at the outermost error, which usually names the operation but not what
/// went wrong in it.
pub fn error_chain(error: &dyn Error) -> String {
    let mut rendered = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        rendered.push_str(": ");
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::error_chain;
    use pretty_assertions::assert_eq;
    use std::io;
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[error("failed to connect")]
    struct Connect(#[source] io::Error);

    #[derive(Debug, Error)]
    #[error("failed to fetch")]
    struct Fetch(#[source] Connect);

    /// Every cause is needed to act on a failure, not only the nearest one.
    #[test]
    fn renders_all_causes_outermost_first() {
        assert_eq!(
            [
                error_chain(&Fetch(Connect(io::Error::other("refused")))),
                error_chain(&io::Error::other("refused")),
            ],
            [
                "failed to fetch: failed to connect: refused".to_owned(),
                "refused".to_owned(),
            ]
        );
    }
}
