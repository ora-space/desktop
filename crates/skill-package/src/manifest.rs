use serde_yaml::Value;

/// The validated public identity extracted from one `SKILL.md` front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub description: String,
}

/// Reports why one `SKILL.md` candidate failed manifest parsing or validation.
///
/// Candidate-level errors only invalidate that candidate; they never reject sibling skills
/// in the same source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// The front matter could not be parsed as a well-formed YAML document.
    YamlInvalid,
    /// The `name` field is missing or blank.
    NameMissing,
    /// The `name` field violates the shared ASCII slug rules.
    NameInvalid,
    /// The `description` field is missing or blank.
    DescriptionMissing,
    /// The trimmed `description` exceeds the 4096-byte limit.
    DescriptionTooLarge,
    /// The manifest file itself exceeds the allowed byte size.
    TooLarge { max_bytes: u64 },
}

impl std::fmt::Display for ManifestError {
    /// Formats one stable candidate failure without exposing source content.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::YamlInvalid => formatter.write_str("invalid YAML front matter"),
            ManifestError::NameMissing => formatter.write_str("missing name"),
            ManifestError::NameInvalid => formatter.write_str("invalid skill name"),
            ManifestError::DescriptionMissing => formatter.write_str("missing description"),
            ManifestError::DescriptionTooLarge => {
                formatter.write_str("description exceeds 4096 bytes")
            }
            ManifestError::TooLarge { max_bytes } => {
                write!(formatter, "manifest exceeds {max_bytes} bytes")
            }
        }
    }
}

/// Maximum UTF-8 bytes a trimmed description may occupy.
pub const MAX_DESCRIPTION_BYTES: usize = 4096;

/// Renders the minimal `SKILL.md` manifest used for ordinary skill creation.
pub fn render_minimal_manifest(name: &str, description: &str) -> String {
    format!("{}---\n", render_front_matter(name, description))
}

/// Rewrites one existing manifest, replacing `name` and `description` while preserving unknown
/// front-matter values and the Markdown body verbatim.
///
/// The front matter is re-serialized, so field order, quoting style, and comments are not
/// guaranteed to survive; the body after the closing fence is kept byte-for-byte. When the
/// existing file has no parseable front matter, a fresh block is prepended and the whole
/// original content is preserved as body.
pub fn rewrite_manifest(
    content: &[u8],
    name: &str,
    description: &str,
) -> Result<String, ManifestError> {
    let text = std::str::from_utf8(content).map_err(|_| ManifestError::YamlInvalid)?;
    match split_front_matter_parts(text) {
        Ok((front_matter, body_start)) => {
            let body = &text[body_start..];
            let mut value: Value = match serde_yaml::from_str(front_matter) {
                Ok(value) => value,
                Err(_) => {
                    return Ok(format!(
                        "{}---\n{text}",
                        render_front_matter(name, description)
                    ));
                }
            };
            let mapping = match value.as_mapping_mut() {
                Some(mapping) => mapping,
                None => {
                    return Ok(format!(
                        "{}---\n{text}",
                        render_front_matter(name, description)
                    ));
                }
            };
            mapping.insert(
                Value::String("name".to_string()),
                Value::String(name.to_string()),
            );
            mapping.insert(
                Value::String("description".to_string()),
                Value::String(description.to_string()),
            );
            let yaml = serde_yaml::to_string(&value).map_err(|_| ManifestError::YamlInvalid)?;
            Ok(format!("---\n{yaml}---\n{body}"))
        }
        // No front matter present: preserve everything as body behind a fresh block.
        Err(ManifestError::NameMissing) => Ok(format!(
            "{}---\n{text}",
            render_front_matter(name, description)
        )),
        Err(error) => Err(error),
    }
}

/// Renders a `---\n<yaml>\n---\n` front-matter block with safe quoting of both values.
fn render_front_matter(name: &str, description: &str) -> String {
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        Value::String("name".to_string()),
        Value::String(name.to_string()),
    );
    mapping.insert(
        Value::String("description".to_string()),
        Value::String(description.to_string()),
    );
    let yaml = serde_yaml::to_string(&Value::Mapping(mapping))
        .unwrap_or_else(|_| format!("name: {name}\ndescription: {description}\n"));
    format!("---\n{yaml}")
}

/// Parses and validates one `SKILL.md` manifest.
///
/// Front matter must start on the first line inside `---` fences. A file without any opening
/// fence reports `NameMissing` because its name field cannot exist; a dangling fence or
/// malformed YAML reports `YamlInvalid`.
pub fn parse_manifest(content: &[u8], max_bytes: u64) -> Result<Manifest, ManifestError> {
    if content.len() as u64 > max_bytes {
        return Err(ManifestError::TooLarge { max_bytes });
    }
    let text = std::str::from_utf8(content).map_err(|_| ManifestError::YamlInvalid)?;
    let (front_matter, _) = split_front_matter_parts(text)?;
    let value: Value =
        serde_yaml::from_str(front_matter).map_err(|_| ManifestError::YamlInvalid)?;
    let mapping = value.as_mapping().ok_or(ManifestError::YamlInvalid)?;

    let name = read_string_field(mapping, "name").ok_or(ManifestError::NameMissing)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(ManifestError::NameMissing);
    }
    if ora_domain::validate_skill_name(name).is_err() {
        return Err(ManifestError::NameInvalid);
    }

    let description =
        read_string_field(mapping, "description").ok_or(ManifestError::DescriptionMissing)?;
    let description = description.trim();
    if description.is_empty() {
        return Err(ManifestError::DescriptionMissing);
    }
    if description.len() > MAX_DESCRIPTION_BYTES {
        return Err(ManifestError::DescriptionTooLarge);
    }

    Ok(Manifest {
        name: name.to_string(),
        description: description.to_string(),
    })
}

/// Reads one string field from the YAML mapping, returning its raw value.
fn read_string_field<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a str> {
    mapping
        .get(Value::String(key.to_string()))
        .and_then(Value::as_str)
}

/// Splits front matter into its YAML body and the byte offset where the Markdown body begins.
fn split_front_matter_parts(text: &str) -> Result<(&str, usize), ManifestError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    if !is_fence(first) {
        return Err(ManifestError::NameMissing);
    }

    let start = first.len();
    let mut position = start;
    for line in lines {
        if is_fence(line) {
            let body_start = position + line.len();
            return Ok((&text[start..position], body_start));
        }
        position += line.len();
    }
    Err(ManifestError::YamlInvalid)
}

/// Detects one `---` fence line while tolerating CRLF line endings.
fn is_fence(line: &str) -> bool {
    line.trim_end_matches(['\r', '\n']).trim_end() == "---"
}
