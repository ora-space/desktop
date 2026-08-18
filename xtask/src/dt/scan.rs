//! Line-level scanner that finds function headers in Rust source text.
//!
//! The scanner does not parse Rust. It tracks the contiguous run of doc
//! comments, plain comments, and attributes that precedes an item ("header
//! block") and reports, for every block, which item terminated it. This is
//! enough for the declaration rules because the DT grammar is line-oriented
//! by design; see the module README for the accepted blind spots.

use super::declaration::{doc_text, is_declaration_attempt};

/// A `/// DT[` line inside a header block, kept verbatim for later parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclarationLine {
    /// 1-based line number in the file.
    pub(crate) line: usize,
    /// Doc text after `///`.
    pub(crate) doc: String,
    /// Whether this line is the very first line of its header block.
    pub(crate) is_first_header_line: bool,
}

/// What ended a header block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HeaderTarget {
    /// A function definition; `is_test` is true when a `*test` attribute is present.
    Function { name: String, is_test: bool },
    /// Any other item line (struct, mod, impl, statement, ...).
    OtherItem,
    /// A blank line or end of file, i.e. the header is attached to nothing.
    Detached,
}

/// A header block together with the item that terminated it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Header {
    /// 1-based line of the terminating item (or of the last header line when detached).
    pub(crate) line: usize,
    pub(crate) declarations: Vec<DeclarationLine>,
    pub(crate) target: HeaderTarget,
}

/// Scans a Rust source file and returns every header block that is worth checking:
/// blocks that terminate at a function, or blocks that carry a declaration attempt.
pub(crate) fn scan_headers(source: &str) -> Vec<Header> {
    let mut headers = Vec::new();
    let mut current = HeaderAccumulator::default();
    // Depth of unbalanced `[` inside a multi-line attribute; zero when not inside one.
    let mut attribute_depth = 0usize;

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = raw_line.trim();

        if attribute_depth > 0 {
            attribute_depth = next_attribute_depth(attribute_depth, trimmed);
            current.saw_line();
            continue;
        }

        if let Some(doc) = doc_text(raw_line) {
            current.push_doc(line_number, doc);
            continue;
        }
        if trimmed.starts_with("//") {
            // Plain comments are transparent inside a header block.
            current.saw_line();
            continue;
        }
        if trimmed.starts_with("#[") {
            if is_test_attribute(trimmed) {
                current.mark_test();
            }
            current.saw_line();
            attribute_depth = next_attribute_depth(0, trimmed);
            continue;
        }
        if trimmed.starts_with("#![") {
            // Inner attributes never belong to a function header, but they may
            // span lines and must be skipped as a unit.
            current.saw_line();
            attribute_depth = next_attribute_depth(0, trimmed);
            continue;
        }

        if trimmed.is_empty() {
            current.finish(&mut headers, line_number, HeaderTarget::Detached);
            continue;
        }
        let target = match function_name(trimmed) {
            Some(name) => HeaderTarget::Function {
                name,
                is_test: current.is_test,
            },
            None => HeaderTarget::OtherItem,
        };
        current.finish(&mut headers, line_number, target);
    }
    let last_line = source.lines().count();
    current.finish(&mut headers, last_line, HeaderTarget::Detached);
    headers
}

/// Mutable state for the header block being accumulated.
#[derive(Debug, Default)]
struct HeaderAccumulator {
    lines_seen: usize,
    is_test: bool,
    declarations: Vec<DeclarationLine>,
}

impl HeaderAccumulator {
    /// Records a doc line, remembering declaration attempts with their position in the block.
    fn push_doc(&mut self, line: usize, doc: &str) {
        if is_declaration_attempt(doc) {
            self.declarations.push(DeclarationLine {
                line,
                doc: doc.to_string(),
                is_first_header_line: self.lines_seen == 0,
            });
        }
        self.lines_seen += 1;
    }

    fn saw_line(&mut self) {
        self.lines_seen += 1;
    }

    fn mark_test(&mut self) {
        self.is_test = true;
    }

    /// Emits the block when it matters and resets for the next one.
    fn finish(&mut self, headers: &mut Vec<Header>, line: usize, target: HeaderTarget) {
        let is_function = matches!(target, HeaderTarget::Function { .. });
        if is_function || !self.declarations.is_empty() {
            headers.push(Header {
                line,
                declarations: std::mem::take(&mut self.declarations),
                target,
            });
        }
        *self = Self::default();
    }
}

/// Updates the bracket depth after consuming one attribute line.
fn next_attribute_depth(depth: usize, line: &str) -> usize {
    line.bytes().fold(depth, |depth, byte| match byte {
        b'[' => depth + 1,
        b']' => depth.saturating_sub(1),
        _ => depth,
    })
}

/// Whether an attribute line names a test macro: its path's last segment is `test`.
fn is_test_attribute(line: &str) -> bool {
    let Some(inner) = line.strip_prefix("#[") else {
        return false;
    };
    let path_end = inner
        .find(|character: char| character == ']' || character == '(' || character.is_whitespace())
        .unwrap_or(inner.len());
    inner[..path_end].rsplit("::").next() == Some("test")
}

/// Extracts the function name when the line starts a function definition.
fn function_name(line: &str) -> Option<String> {
    let mut rest = line;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("fn") {
            let after = after.strip_prefix(char::is_whitespace)?;
            let name: String = after
                .chars()
                .take_while(|character| character.is_alphanumeric() || *character == '_')
                .collect();
            return (!name.is_empty()).then_some(name);
        }
        // Visibility and qualifiers that may precede `fn`.
        rest = if let Some(after) = rest.strip_prefix("pub(") {
            after.split_once(')')?.1
        } else {
            let (word, after) = rest.split_once(char::is_whitespace)?;
            match word {
                "pub" | "async" | "unsafe" | "const" | "extern" | "default" => after,
                _ if word.starts_with("extern") || word.starts_with('"') => after,
                _ => return None,
            }
        };
    }
}
