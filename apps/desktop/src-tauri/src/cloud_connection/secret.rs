//! The session token as a value that cannot leak through formatting.

/// A Cloud desktop token. `Debug` prints a placeholder so the token cannot reach a log line,
/// diagnostic or panic message through a derived `Debug` of an enclosing type.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionToken(String);

impl SessionToken {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    /// The raw value, for the Authorization header and the secure store only.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SessionToken {
    /// Never renders the token.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SessionToken(***)")
    }
}
