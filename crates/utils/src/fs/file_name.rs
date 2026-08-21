use crate::path::is_windows_reserved_device_name;
use std::ffi::OsStr;
use std::path::Path;

/// Longest stem kept after sanitization. Long enough for descriptive download names while
/// leaving room below common 255-byte filename limits for a numeric suffix and an extension.
const MAX_FILE_STEM_BYTES: usize = 120;

/// Produces one portable basename from untrusted text while preserving its extension.
///
/// Directory components are dropped so a suggestion such as `../nested/evil.zip` cannot escape
/// the target directory; separators, control characters, and Windows-illegal punctuation become
/// `_` so the same bytes are valid on every supported host. `fallback_stem` is used when nothing
/// usable remains (empty input, dots only, or a name that sanitizes to nothing).
pub fn sanitize_file_name(candidate: &str, fallback_stem: &str) -> String {
    let basename = Path::new(candidate)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    // Path::file_stem treats a leading dot as part of the stem (".bashrc" has no extension),
    // which matches the convention documented in the module README.
    let raw_stem = Path::new(basename)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(basename);
    let raw_extension = Path::new(basename).extension().and_then(OsStr::to_str);

    let stem = sanitize_component(raw_stem, MAX_FILE_STEM_BYTES);
    let stem = if stem.is_empty() {
        fallback_stem.to_owned()
    } else if is_windows_reserved_device_name(&stem) {
        // `CON.zip` still resolves to the console device on Windows, so the stem is prefixed
        // rather than rejected to keep the caller's flow simple.
        format!("_{stem}")
    } else {
        stem
    };

    match raw_extension.map(|extension| sanitize_component(extension, MAX_FILE_STEM_BYTES)) {
        Some(extension) if !extension.is_empty() => format!("{stem}.{extension}"),
        Some(_) | None => stem,
    }
}

/// Replaces unsafe characters, trims edge spaces and dots, and truncates on a character boundary.
///
/// Shared by the stem and the extension because both sides of the final dot face the same
/// per-host restrictions.
fn sanitize_component(value: &str, max_bytes: usize) -> String {
    let mut sanitized = String::new();
    for character in value.trim_matches([' ', '.']).chars() {
        let character = if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
            '_'
        } else {
            character
        };
        if sanitized.len() + character.len_utf8() > max_bytes {
            break;
        }
        sanitized.push(character);
    }
    sanitized.trim_matches([' ', '.']).to_owned()
}

#[cfg(test)]
mod tests {
    use super::{MAX_FILE_STEM_BYTES, sanitize_file_name};
    use pretty_assertions::assert_eq;

    /// Verifies path components, controls, illegal characters, empty names, and devices are safe
    /// while the original extension survives untouched.
    #[test]
    fn sanitizes_untrusted_file_names() {
        // Windows Path::file_name treats backslashes as separators; Unix keeps them in the stem.
        let windows_drive_path = if cfg!(windows) {
            "evil.zip"
        } else {
            "C__nested_evil.zip"
        };
        let cases = [
            ("", "download"),
            ("skill.zip", "skill.zip"),
            ("Mixed.Zip", "Mixed.Zip"),
            ("My Skill (1).zip", "My Skill (1).zip"),
            ("pack.tar.gz", "pack.tar.gz"),
            ("README", "README"),
            (".bashrc", "bashrc"),
            ("../nested/unsafe?name.ZIP", "unsafe_name.ZIP"),
            ("C:\\nested\\evil.zip", windows_drive_path),
            ("bad\u{7}name.zip", "bad_name.zip"),
            ("...zip", "download.zip"),
            ("...", "download"),
            ("CON.zip", "_CON.zip"),
            ("lpt1", "_lpt1"),
            ("a<b>c:d.zip", "a_b_c_d.zip"),
            ("trailing. .txt", "trailing.txt"),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_file_name(input, "download"), expected, "{input}");
        }
    }

    /// Verifies an oversized stem is cut on a character boundary and never exceeds the limit.
    #[test]
    fn truncates_long_stems_on_character_boundaries() {
        // Each `é` is two bytes, so 61 of them exceed 120 bytes by one byte.
        let long = format!("{}.zip", "é".repeat(61));
        let sanitized = sanitize_file_name(&long, "download");
        assert_eq!(sanitized, format!("{}.zip", "é".repeat(60)));
        assert_eq!(sanitized.len(), MAX_FILE_STEM_BYTES + ".zip".len());
    }
}
