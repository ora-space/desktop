use ora_contracts::{TaskDiffCommentAnchor, TaskDiffSide};

/// Verifies that a client-provided anchor identifies real lines in one generated patch.
pub(super) fn anchor_exists(patch: &str, anchor: &TaskDiffCommentAnchor) -> bool {
    patch
        .split("diff --git ")
        .skip(1)
        .any(|section| section_contains_anchor(section, anchor))
}

/// Restricts hunk and line matching to the file named by the anchor.
fn section_contains_anchor(section: &str, anchor: &TaskDiffCommentAnchor) -> bool {
    let marker = match anchor.side {
        TaskDiffSide::Old => "--- ",
        TaskDiffSide::New => "+++ ",
    };
    if !section
        .lines()
        .find_map(|line| line.strip_prefix(marker))
        .is_some_and(|path| patch_path_matches(path, &anchor.path))
    {
        return false;
    }

    let lines: Vec<_> = section.lines().collect();
    lines.iter().enumerate().any(|(index, line)| {
        *line == anchor.hunk_header && hunk_contains_anchor(&lines[index + 1..], anchor)
    })
}

/// Walks one hunk's old/new counters and verifies every selected line plus its first-line content.
fn hunk_contains_anchor(lines: &[&str], anchor: &TaskDiffCommentAnchor) -> bool {
    let Some((mut old_line, mut new_line)) = parse_hunk_starts(&anchor.hunk_header) else {
        return false;
    };
    let mut matched_lines = 0_u32;

    for line in lines {
        if line.starts_with("@@ ") || line.starts_with("diff --git ") {
            break;
        }
        let (line_number, content) = match (line.chars().next(), anchor.side) {
            (Some(' '), TaskDiffSide::Old) => (Some(old_line), &line[1..]),
            (Some(' '), TaskDiffSide::New) => (Some(new_line), &line[1..]),
            (Some('-'), TaskDiffSide::Old) => (Some(old_line), &line[1..]),
            (Some('+'), TaskDiffSide::New) => (Some(new_line), &line[1..]),
            _ => (None, ""),
        };
        if let Some(line_number) = line_number
            && (anchor.start_line..=anchor.end_line).contains(&line_number)
        {
            if line_number == anchor.start_line && content != anchor.line_content {
                return false;
            }
            matched_lines += 1;
        }
        match line.chars().next() {
            Some(' ') => {
                old_line += 1;
                new_line += 1;
            }
            Some('-') => old_line += 1,
            Some('+') => new_line += 1,
            _ => {}
        }
    }

    matched_lines == anchor.end_line - anchor.start_line + 1
}

/// Extracts the old and new starting counters from a standard unified hunk header.
fn parse_hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace();
    (parts.next()? == "@@").then_some(())?;
    let old = parse_range_start(parts.next()?, '-')?;
    let new = parse_range_start(parts.next()?, '+')?;
    Some((old, new))
}

/// Parses the first line number from one `-start,count` or `+start,count` range.
fn parse_range_start(range: &str, prefix: char) -> Option<u32> {
    range.strip_prefix(prefix)?.split(',').next()?.parse().ok()
}

/// Matches ordinary Git paths and the quoted representation used for unusual filenames.
fn patch_path_matches(patch_path: &str, expected: &str) -> bool {
    let decoded = if patch_path.starts_with('"') {
        decode_quoted_path(patch_path)
    } else {
        Some(patch_path.to_string())
    };
    decoded
        .as_deref()
        .and_then(|path| path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")))
        == Some(expected)
}

/// Decodes Git's double-quoted path form, including octal UTF-8 byte escapes.
fn decode_quoted_path(value: &str) -> Option<String> {
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut bytes = Vec::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '\\' | '"' => bytes.push(escaped as u8),
            'a' => bytes.push(0x07),
            'b' => bytes.push(0x08),
            'v' => bytes.push(0x0b),
            'f' => bytes.push(0x0c),
            't' => bytes.push(b'\t'),
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            '0'..='7' => {
                let mut octal = String::from(escaped);
                octal.push(chars.next()?);
                octal.push(chars.next()?);
                bytes.push(u8::from_str_radix(&octal, 8).ok()?);
            }
            _ => return None,
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::{anchor_exists, decode_quoted_path};
    use ora_contracts::{TaskDiffCommentAnchor, TaskDiffSide};
    use pretty_assertions::assert_eq;

    /// Rejects lines and content that do not occur in the selected file and hunk.
    #[test]
    fn validates_real_patch_anchors() {
        let patch = "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,1 +1,2 @@\n old\n+new\n";
        let anchor = TaskDiffCommentAnchor {
            diff_id: "diff".to_string(),
            path: "src/main.rs".to_string(),
            side: TaskDiffSide::New,
            start_line: 2,
            end_line: 2,
            hunk_header: "@@ -1,1 +1,2 @@".to_string(),
            line_content: "new".to_string(),
        };

        assert_eq!(anchor_exists(patch, &anchor), true);
        assert_eq!(
            anchor_exists(
                patch,
                &TaskDiffCommentAnchor {
                    start_line: 99,
                    end_line: 99,
                    ..anchor
                }
            ),
            false
        );
    }

    /// Covers every single-character C escape emitted by Git's path quoting rules.
    #[test]
    fn decodes_git_control_character_escapes() {
        assert_eq!(
            decode_quoted_path("\"a/\\a\\b\\t\\n\\v\\f\\r\\\\\\\"\""),
            Some("a/\x07\x08\t\n\x0b\x0c\r\\\"".to_string())
        );
    }
}
