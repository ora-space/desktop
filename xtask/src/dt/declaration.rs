//! Parsing of a single `/// DT[<feature>][<kind>] <statement>` line.
//!
//! The grammar is deliberately line-local so that a declaration can be
//! recognised without any Rust syntax analysis. Only the text after `///`
//! is inspected here; deciding which function a line belongs to is the
//! scanner's job.

use std::fmt;

/// Reserved word that may replace the feature or the kind when a test cannot be
/// classified yet. It never appears in a README catalog.
pub(crate) const TODO: &str = "todo";

/// Prefix that marks a doc line as a DT declaration attempt.
const PREFIX: &str = "DT[";

/// Coverage region a test verifies. Closed set; `Todo` is the governance placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Happy,
    Edge,
    Error,
    Concurrency,
    Todo,
}

impl Kind {
    /// Maps the textual kind to the enum; unknown words are rejected by the caller.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "happy" => Some(Self::Happy),
            "edge" => Some(Self::Edge),
            "error" => Some(Self::Error),
            "concurrency" => Some(Self::Concurrency),
            TODO => Some(Self::Todo),
            _ => None,
        }
    }
}

/// Feature point reference as written in the declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FeatureRef {
    /// Reserved placeholder; resolves against no catalog.
    Todo,
    /// Kebab-case id resolved against the owning README of the test file.
    Local(String),
    /// Id resolved against `src/<segments...>/README.md` of the same crate.
    Qualified { segments: Vec<String>, id: String },
}

impl fmt::Display for FeatureRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Todo => formatter.write_str(TODO),
            Self::Local(id) => formatter.write_str(id),
            Self::Qualified { segments, id } => {
                for segment in segments {
                    write!(formatter, "{segment}::")?;
                }
                formatter.write_str(id)
            }
        }
    }
}

/// A syntactically valid declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub(crate) feature: FeatureRef,
    pub(crate) kind: Kind,
    pub(crate) statement: String,
}

/// Why a `DT[` line failed to parse. Each variant maps to one human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParseError {
    Malformed(&'static str),
    UnknownKind(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(reason) => write!(
                formatter,
                "malformed declaration ({reason}); expected `/// DT[<feature>][<kind>] <statement>`"
            ),
            Self::UnknownKind(kind) => write!(
                formatter,
                "unknown kind `{kind}`; expected one of happy, edge, error, concurrency, todo"
            ),
        }
    }
}

/// Returns the doc text of a `///` line, or `None` when the line is not an outer doc comment.
pub(crate) fn doc_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    // `////` is a plain comment in Rust, not a doc comment.
    if trimmed.starts_with("////") {
        return None;
    }
    trimmed.strip_prefix("///")
}

/// Whether a doc line is a declaration attempt, i.e. it must parse or be reported.
pub(crate) fn is_declaration_attempt(doc: &str) -> bool {
    doc.trim_start().starts_with(PREFIX)
}

/// Parses the doc text (the part after `///`) of a declaration attempt.
pub(crate) fn parse_declaration(doc: &str) -> Result<Declaration, ParseError> {
    let rest = doc
        .trim_start()
        .strip_prefix(PREFIX)
        .ok_or(ParseError::Malformed("missing `DT[` prefix"))?;
    let (feature_text, rest) = rest
        .split_once(']')
        .ok_or(ParseError::Malformed("unterminated feature field"))?;
    let rest = rest.strip_prefix('[').ok_or(ParseError::Malformed(
        "kind field must directly follow the feature field",
    ))?;
    let (kind_text, rest) = rest
        .split_once(']')
        .ok_or(ParseError::Malformed("unterminated kind field"))?;
    let statement = rest.strip_prefix(' ').ok_or(ParseError::Malformed(
        "statement must be separated by one space",
    ))?;

    let feature = parse_feature(feature_text)?;
    let kind =
        Kind::parse(kind_text).ok_or_else(|| ParseError::UnknownKind(kind_text.to_string()))?;
    if statement.is_empty() || statement.starts_with(char::is_whitespace) {
        return Err(ParseError::Malformed(
            "statement must not be empty or start with whitespace",
        ));
    }
    if statement.ends_with(char::is_whitespace) {
        return Err(ParseError::Malformed(
            "statement must not end with whitespace",
        ));
    }

    Ok(Declaration {
        feature,
        kind,
        statement: statement.to_string(),
    })
}

/// Parses `todo`, `id`, or `seg::seg::id`.
fn parse_feature(text: &str) -> Result<FeatureRef, ParseError> {
    if text == TODO {
        return Ok(FeatureRef::Todo);
    }
    let mut parts: Vec<&str> = text.split("::").collect();
    let id = parts
        .pop()
        .filter(|id| is_feature_id(id))
        .ok_or(ParseError::Malformed(
            "feature id must be kebab-case: [a-z0-9]+(-[a-z0-9]+)*",
        ))?;
    if id == TODO {
        return Err(ParseError::Malformed("`todo` cannot be module-qualified"));
    }
    if parts.iter().any(|segment| !is_module_segment(segment)) {
        return Err(ParseError::Malformed(
            "module qualifier segments must be Rust identifiers separated by `::`",
        ));
    }
    if parts.is_empty() {
        return Ok(FeatureRef::Local(id.to_string()));
    }
    Ok(FeatureRef::Qualified {
        segments: parts.iter().map(ToString::to_string).collect(),
        id: id.to_string(),
    })
}

/// Checks the kebab-case id grammar shared by declarations and README catalogs.
pub(crate) fn is_feature_id(text: &str) -> bool {
    !text.is_empty()
        && text.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// Checks a Rust module path segment: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_module_segment(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
