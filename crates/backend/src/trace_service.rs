//! The only component that touches trace files: the host-side read service.
//!
//! The dashboard never sees a path. It calls the host methods that land here, the service
//! resolves the locator through [`TraceRegistry`], reads only the requested window, and returns
//! bytes plus a byte offset — no path ever crosses the boundary.

use crate::trace_registry::{ResolvedTrace, TraceRegistry};
use ora_domain::AgentRef;
use ora_plugin_manifest::TraceLocator;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

/// One read chunk is capped at 1 MiB so the bridge response stays well under its limit.
pub const TRACE_CHUNK_MAX_BYTES: usize = 1024 * 1024;
/// Session lists cap at a generous bound; the page paginates.
const LIST_MAX_ENTRIES: usize = 5000;
/// Only the most recent entries get the head scan that extracts a readable session name.
const LIST_NAME_SCAN_ENTRIES: usize = 300;
/// Head-scan byte cap for name extraction; large files are never read in full for a label.
const NAME_HEAD_SCAN_BYTES: usize = 256 * 1024;
/// Readable names truncate at this many characters plus an ellipsis.
const NAME_MAX_CHARS: usize = 40;
/// A child session is readable only when its id appears inside the parent trace; this caps how
/// much of the parent is scanned for that check.
const CHILD_MATCH_SCAN_BYTES: usize = 8 * 1024 * 1024;
/// Directory walks stop after this many entries; a trace root with more files is pathological.
const MAX_WALK_ENTRIES: usize = 10_000;

/// Snapshot of one trace file, cheap enough to poll (metadata only, never a full read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceStat {
    /// Passthrough parser selector from the declaration (`claude_code`, `opencode`, …).
    pub format: String,
    /// Whether the file exists yet; a session that just started may not have written one.
    pub exists: bool,
    pub size_bytes: u64,
    pub mtime_ms: u64,
}

/// One line-aligned window of trace text plus the offset to continue from.
///
/// `next_offset` is a byte offset, not a line number: a caller polls by passing it back. A line
/// longer than the chunk is returned in parts; consumers buffer a partial tail until a newline
/// arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceChunk {
    pub text: String,
    pub next_offset: u64,
    /// True once `next_offset` reached the end of the file.
    pub done: bool,
}

/// One entry of the session list, with the readable name extracted by the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceEntryMeta {
    pub agent: String,
    pub session_id: String,
    pub name: Option<String>,
    pub mtime_ms: u64,
    pub size_bytes: u64,
}

/// Resolves and reads trace files on the host's behalf.
pub struct TraceService {
    registry: Arc<TraceRegistry>,
    home: PathBuf,
    data_dir: PathBuf,
    /// Memoized (agent, session) → located file. A failed `stat` drops the entry so a file that
    /// appears later (Claude writes the transcript shortly after the session starts) is found.
    located: Mutex<HashMap<(AgentRef, String), PathBuf>>,
}

impl TraceService {
    /// Builds the service over a registry; `home`/`data_dir` feed placeholder substitution.
    pub fn new(registry: Arc<TraceRegistry>, home: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            registry,
            home,
            data_dir,
            located: Mutex::new(HashMap::new()),
        }
    }

    /// Returns one session's trace stat, or `None` when the agent has no declaration.
    pub fn stat(&self, agent: &AgentRef, session_id: &str) -> Option<TraceStat> {
        let resolved = self.resolve(agent, session_id)?;
        let Some(path) = self.resolve_file(agent, session_id, &resolved) else {
            return Some(TraceStat {
                format: resolved.format,
                exists: false,
                size_bytes: 0,
                mtime_ms: 0,
            });
        };
        let metadata = fs::metadata(&path).ok();
        Some(TraceStat {
            format: resolved.format,
            exists: metadata.is_some(),
            size_bytes: metadata.as_ref().map(std::fs::Metadata::len).unwrap_or(0),
            mtime_ms: metadata
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0),
        })
    }

    /// Reads one line-aligned window starting at `offset`, capped at `max_bytes`.
    pub fn read(
        &self,
        agent: &AgentRef,
        session_id: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Option<TraceChunk> {
        let resolved = self.resolve(agent, session_id)?;
        let path = self.resolve_file(agent, session_id, &resolved)?;
        read_window(&path, offset, max_bytes)
    }

    /// Reads a child session's trace after verifying the child id appears inside the parent.
    pub fn read_child(
        &self,
        agent: &AgentRef,
        parent_session_id: &str,
        child_session_id: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Option<TraceChunk> {
        let parent = self.resolve(agent, parent_session_id)?;
        let parent_path = self.resolve_file(agent, parent_session_id, &parent)?;
        if !contains_token(&parent_path, child_session_id.as_bytes()) {
            return None;
        }
        self.read(agent, child_session_id, offset, max_bytes)
    }

    /// Whether one session file exists under the agent's declared roots.
    ///
    /// Membership is what gates "read by listed id" in `TraceHost`: the caller may only name a
    /// session that the listing can show, never an arbitrary path-derived id.
    pub fn has_session(&self, agent: &AgentRef, session_id: &str) -> bool {
        let Some(resolved) = self.resolve(agent, session_id) else {
            return false;
        };
        self.resolve_file(agent, session_id, &resolved).is_some()
    }

    /// Lists trace sessions for every declared agent (or one agent), newest first.
    pub fn list(&self, agent_filter: Option<&AgentRef>) -> Vec<TraceEntryMeta> {
        let mut entries: Vec<TraceEntryMeta> = Vec::new();
        for agent in self.registry.agents() {
            if agent_filter.is_some_and(|filter| filter != &agent) {
                continue;
            }
            let Some(declaration) = self.registry.declaration(&agent) else {
                continue;
            };
            let Ok(locator) = declaration.resolve_listing(&self.home, &self.data_dir) else {
                continue;
            };
            let scan = match locator {
                TraceLocator::File { path } => {
                    let Some(directory) = path.parent() else {
                        continue;
                    };
                    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    scan_directory(&agent, directory, file_name, false)
                }
                TraceLocator::Search { root, pattern } => {
                    scan_directory(&agent, &root, &pattern, true)
                }
            };
            entries.extend(scan);
        }

        entries.sort_by(|left, right| {
            right
                .mtime_ms
                .cmp(&left.mtime_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        entries.truncate(LIST_MAX_ENTRIES);
        for entry in entries.iter_mut().take(LIST_NAME_SCAN_ENTRIES) {
            entry.name = self.extract_name(&entry.agent, &entry.session_id);
        }
        entries
    }

    /// Resolves the declaration for one session through the registry.
    fn resolve(&self, agent: &AgentRef, session_id: &str) -> Option<ResolvedTrace> {
        self.registry.resolve(
            agent,
            &ora_plugin_manifest::TraceResolveContext {
                home: &self.home,
                data_dir: &self.data_dir,
                agent_session_id: session_id,
            },
        )
    }

    /// Locates the file for a resolved declaration, memoizing successful lookups.
    fn resolve_file(
        &self,
        agent: &AgentRef,
        session_id: &str,
        resolved: &ResolvedTrace,
    ) -> Option<PathBuf> {
        let key = (agent.clone(), session_id.to_owned());
        if let Some(cached) = self
            .located
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            && cached.is_file()
        {
            return Some(cached.clone());
        }
        let path = match &resolved.locator {
            TraceLocator::File { path } => {
                if path.is_file() {
                    path.clone()
                } else {
                    return None;
                }
            }
            TraceLocator::Search { root, pattern } => find_match(root, pattern)?,
        };
        self.located
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, path.clone());
        Some(path)
    }

    /// Extracts the readable name for a listed session, resolving through the same path used by
    /// reads so the cache is shared.
    fn extract_name(&self, agent: &str, session_id: &str) -> Option<String> {
        let agent_ref = AgentRef::parse(agent).ok()?;
        let resolved = self.resolve(&agent_ref, session_id)?;
        let path = self.resolve_file(&agent_ref, session_id, &resolved)?;
        extract_name_from_path(&path, &resolved.format)
    }
}

/// Reads one line-aligned window; see [`TraceChunk`] for the offset contract.
fn read_window(path: &Path, offset: u64, max_bytes: usize) -> Option<TraceChunk> {
    let cap = max_bytes.min(TRACE_CHUNK_MAX_BYTES);
    let mut file = fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let start = offset.min(file_len);
    if start == file_len {
        return Some(TraceChunk {
            text: String::new(),
            next_offset: start,
            done: true,
        });
    }

    // The boundary probe above moved the cursor; seek back before reading the window.
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buffer = vec![0u8; cap];
    let mut read = 0usize;
    while read < cap {
        let count = match file.read(&mut buffer[read..]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => return None,
        };
        read += count;
    }
    let window = &buffer[..read];
    let eof = start + read as u64 >= file_len;

    // Every non-EOF chunk ends exactly after a newline, so a resuming read normally starts at a
    // line boundary; only a line longer than one chunk resumes mid-line. Check the byte before
    // `start` to tell the two apart, and drop the partial first line only when resuming mid-line.
    let mut previous = [0u8; 1];
    let mid_line = if start > 0 {
        file.seek(SeekFrom::Start(start - 1))
            .ok()
            .and_then(|_| file.read(&mut previous).ok())
            .map(|count| count == 1 && previous[0] != b'\n')
            .unwrap_or(false)
    } else {
        false
    };
    let from = if mid_line {
        match window.iter().position(|byte| *byte == b'\n') {
            Some(position) => position + 1,
            None => {
                return Some(TraceChunk {
                    text: String::new(),
                    next_offset: start + read as u64,
                    done: eof,
                });
            }
        }
    } else {
        0
    };

    // Trim the tail back to a complete line unless this window reaches the end of the file.
    let to = if eof {
        window.len()
    } else {
        window[from..]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|position| from + position + 1)
            .unwrap_or(window.len())
    };

    Some(TraceChunk {
        text: String::from_utf8_lossy(&window[from..to]).into_owned(),
        next_offset: start + to as u64,
        done: eof,
    })
}

/// Scans `directory` (recursively when `recursive`) for files matching one session pattern.
///
/// The pattern already has its `{agent_session_id}` replaced by `*` (`resolve_listing`), so
/// every matched file yields one entry whose session id is the wildcard match.
fn scan_directory(
    agent: &AgentRef,
    directory: &Path,
    pattern: &str,
    recursive: bool,
) -> Vec<TraceEntryMeta> {
    if !directory.is_dir() {
        return Vec::new();
    }
    let mut entries = Vec::new();
    walk(directory, recursive, |relative, metadata| {
        let Some((session_id, _file_name)) = match_session(relative, pattern) else {
            return;
        };
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        entries.push(TraceEntryMeta {
            agent: agent.as_str().to_owned(),
            session_id,
            name: None,
            mtime_ms,
            size_bytes: metadata.len(),
        });
    });
    entries
}

/// Walks a directory tree with an entry budget; `visit` receives each file's path relative to
/// `directory` (slash-separated) and its metadata.
fn walk(directory: &Path, recursive: bool, mut visit: impl FnMut(&str, &fs::Metadata)) {
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(directory.to_path_buf(), PathBuf::new())];
    let mut seen = 0usize;
    while let Some((current, relative)) = stack.pop() {
        let Ok(items) = fs::read_dir(&current) else {
            continue;
        };
        for item in items.filter_map(Result::ok) {
            seen += 1;
            if seen > MAX_WALK_ENTRIES {
                return;
            }
            let path = item.path();
            let name = item.file_name();
            let mut child_relative = relative.clone();
            child_relative.push(name);
            let file_type = match item.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_file() {
                if let Ok(metadata) = item.metadata() {
                    let relative = child_relative.to_string_lossy().replace('\\', "/");
                    visit(&relative, &metadata);
                }
            } else if recursive && file_type.is_dir() {
                stack.push((path, child_relative));
            }
        }
    }
}

/// Extracts the session id from one matched relative path, or `None` when it does not match.
///
/// The pattern is the declared template with `{agent_session_id}` replaced by `*`; the matched
/// filename therefore has the shape `prefix + session_id + suffix`, so the id is the middle.
fn match_session(relative: &str, pattern: &str) -> Option<(String, String)> {
    if !matches_pattern(pattern, relative) {
        return None;
    }
    let file_name = relative.rsplit('/').next()?;
    let pattern_file = pattern.rsplit('/').next()?;
    let star = pattern_file.find('*')?;
    let prefix = &pattern_file[..star];
    let suffix = &pattern_file[star + 1..];
    let stem = file_name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if stem.is_empty() {
        return None;
    }
    Some((stem.to_owned(), file_name.to_owned()))
}

/// Matches a slash-separated relative path against a pattern with `*` (one segment part) and
/// `**` (zero or more whole segments) wildcards.
fn matches_pattern(pattern: &str, relative: &str) -> bool {
    let parts: Vec<&str> = pattern.split('/').collect();
    let texts: Vec<&str> = relative.split('/').collect();
    match_parts(&parts, &texts)
}

fn match_parts(parts: &[&str], texts: &[&str]) -> bool {
    if parts.is_empty() {
        return texts.is_empty();
    }
    match parts[0] {
        "**" => {
            for skip in 0..=texts.len() {
                if match_parts(&parts[1..], &texts[skip..]) {
                    return true;
                }
            }
            false
        }
        part => {
            if texts.is_empty() || !segment_match(part, texts[0]) {
                return false;
            }
            match_parts(&parts[1..], &texts[1..])
        }
    }
}

fn segment_match(pattern: &str, text: &str) -> bool {
    let mut pattern_chars = pattern.chars();
    let mut text_chars = text.chars();
    let mut pattern_char = pattern_chars.next();
    let mut text_char = text_chars.next();
    while pattern_char.is_some() {
        if pattern_char == Some('*') {
            let Some(next) = pattern_chars.next() else {
                return true;
            };
            pattern_char = Some(next);
            while text_char.is_some() && text_char != pattern_char {
                text_char = text_chars.next();
            }
            if text_char.is_none() {
                return false;
            }
        } else if pattern_char == text_char {
            pattern_char = pattern_chars.next();
            text_char = text_chars.next();
        } else {
            return false;
        }
    }
    text_char.is_none()
}

/// Finds the first file under `root` matching the pattern.
fn find_match(root: &Path, pattern: &str) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(current) = stack.pop() {
        let Ok(items) = fs::read_dir(&current) else {
            continue;
        };
        for item in items.filter_map(Result::ok) {
            seen += 1;
            if seen > MAX_WALK_ENTRIES {
                return None;
            }
            let path = item.path();
            let file_type = item.file_type().ok()?;
            if file_type.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                if matches_pattern(pattern, &relative) {
                    return Some(path);
                }
            } else if file_type.is_dir() {
                stack.push(path);
            }
        }
    }
    None
}

/// Scans the first bytes of a file for a byte token; the child-session check.
fn contains_token(path: &Path, token: &[u8]) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buffer = [0u8; 64 * 1024];
    let mut scanned = 0usize;
    while scanned < CHILD_MATCH_SCAN_BYTES {
        let count = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => return false,
        };
        scanned += count;
        if find_bytes(&buffer[..count], token) {
            return true;
        }
    }
    false
}

/// Naive substring search over bytes; tokens are short session ids.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Extracts a readable session name from a trace file head.
///
/// - `claude_code`: the first real user text (fallback: the cwd directory name).
/// - `opencode`: the `session.start` title (fallback: the file stem).
/// - Anything else: the file stem.
fn extract_name_from_path(path: &Path, format: &str) -> Option<String> {
    let Ok(file) = fs::File::open(path) else {
        return None;
    };
    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();
    let mut scanned = 0usize;
    let mut cwd: Option<String> = None;
    let mut title: Option<String> = None;
    while let Ok(count) = reader.read_line(&mut line) {
        if count == 0 {
            break;
        }
        scanned += line.len();
        if scanned > NAME_HEAD_SCAN_BYTES {
            break;
        }
        if line.trim().is_empty() {
            line.clear();
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            line.clear();
            continue;
        };
        line.clear();
        if cwd.is_none()
            && let Some(found) = value.get("cwd").and_then(|found| found.as_str())
            && !found.trim().is_empty()
        {
            cwd = Some(found.trim().to_owned());
        }
        match value.get("type").and_then(|found| found.as_str()) {
            Some("session.start") if format == "opencode" => {
                title = value
                    .get("title")
                    .and_then(|found| found.as_str())
                    .map(str::trim)
                    .filter(|found| !found.is_empty())
                    .map(str::to_owned);
                break;
            }
            Some("user") if format == "claude_code" => {
                let message = value.get("message").unwrap_or(&value);
                let content = message.get("content").unwrap_or(&serde_json::Value::Null);
                if let Some(found) = content.as_str() {
                    let found = found.trim();
                    if !found.is_empty() {
                        return Some(truncate_name(found));
                    }
                    continue;
                }
                if let Some(blocks) = content.as_array() {
                    for block in blocks {
                        if block.get("type").and_then(|found| found.as_str()) != Some("text") {
                            continue;
                        }
                        if let Some(found) = block.get("text").and_then(|found| found.as_str()) {
                            let found = found.trim();
                            if !found.is_empty() {
                                return Some(truncate_name(found));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if format == "opencode" {
        return title.or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(truncate_name)
        });
    }
    cwd.and_then(|found| {
        Path::new(&found)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(truncate_name)
    })
}

fn truncate_name(value: &str) -> String {
    let mut characters = value.chars();
    let mut output: String = characters.by_ref().take(NAME_MAX_CHARS).collect();
    if characters.next().is_some() {
        output.push('…');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_plugin_manifest::PluginAgentTrace;
    use tempfile::tempdir;

    /// Extracts an expected successful result without using `expect` in tests.
    fn must<T, E: std::fmt::Debug>(result: Result<T, E>, label: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("expected {label} to succeed, got {error:?}"),
        }
    }

    /// Extracts an expected resolved value without using `expect` in tests.
    fn must_some<T>(value: Option<T>, label: &str) -> T {
        match value {
            Some(value) => value,
            None => panic!("expected {label} to resolve"),
        }
    }

    /// Builds a service whose home/data_dir live inside one temp dir.
    fn service_with(declaration: PluginAgentTrace) -> (TraceService, tempfile::TempDir, AgentRef) {
        let temp = tempdir().expect("tempdir");
        let registry = Arc::new(TraceRegistry::new(Vec::new()));
        let agent = must(AgentRef::parse("ora-space.test"), "agent ref");
        registry.register_plugin(agent.clone(), declaration);
        let service =
            TraceService::new(registry, temp.path().join("home"), temp.path().join("data"));
        (service, temp, agent)
    }

    /// Builds a file-form declaration rooted in the service's data dir.
    fn file_declaration() -> PluginAgentTrace {
        must(
            PluginAgentTrace::file(
                "opencode",
                "{data_dir}/opencode/trace/{agent_session_id}.ndjson",
            ),
            "file declaration",
        )
    }

    #[test]
    fn stat_reports_missing_and_present_files() {
        let (service, temp, agent) = service_with(file_declaration());
        let trace_dir = temp.path().join("data/opencode/trace");
        fs::create_dir_all(&trace_dir).expect("trace dir");

        let missing = must_some(service.stat(&agent, "ses_1"), "stat for missing file");
        assert_eq!(missing.exists, false);
        assert_eq!(missing.format, "opencode");

        let path = trace_dir.join("ses_1.ndjson");
        fs::write(&path, "{}\n").expect("write trace");

        let present = must_some(service.stat(&agent, "ses_1"), "stat for present file");
        assert!(present.exists);
        assert_eq!(present.size_bytes, 3);
        assert!(present.mtime_ms > 0);
    }

    #[test]
    fn read_windows_are_line_aligned_and_resumable() {
        let (service, temp, agent) = service_with(file_declaration());
        let trace_dir = temp.path().join("data/opencode/trace");
        fs::create_dir_all(&trace_dir).expect("trace dir");
        let path = trace_dir.join("ses_1.ndjson");
        let lines: Vec<String> = (0..10)
            .map(|index| format!("{{\"line\":{index}}}\n"))
            .collect();
        fs::write(&path, lines.concat()).expect("write trace");

        let first = must_some(service.read(&agent, "ses_1", 0, 24), "first chunk");
        assert_eq!(first.text, "{\"line\":0}\n{\"line\":1}\n");
        assert!(!first.done);

        let second = must_some(
            service.read(&agent, "ses_1", first.next_offset, 1024),
            "second chunk",
        );
        assert!(second.text.starts_with("{\"line\":2}\n"));
        assert!(second.done);

        // Reading past the end is a completed empty chunk, not an error.
        let past = must_some(
            service.read(&agent, "ses_1", 10_000, 1024),
            "past-the-end chunk",
        );
        assert!(past.text.is_empty() && past.done);
    }

    #[test]
    fn read_tracks_file_growth_through_offsets() {
        let (service, temp, agent) = service_with(file_declaration());
        let trace_dir = temp.path().join("data/opencode/trace");
        fs::create_dir_all(&trace_dir).expect("trace dir");
        let path = trace_dir.join("ses_1.ndjson");
        fs::write(&path, "{\"a\":1}\n").expect("initial write");

        let first = must_some(service.read(&agent, "ses_1", 0, 1024), "first chunk");
        assert!(first.done);

        // The file grows after the first read; the same offset yields the new line.
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open append");
        writeln!(file, "{{\"a\":2}}").expect("append line");

        let second = must_some(
            service.read(&agent, "ses_1", first.next_offset, 1024),
            "chunk after growth",
        );
        assert_eq!(second.text, "{\"a\":2}\n");
        assert!(second.done);
    }

    #[test]
    fn search_form_locates_files_by_session_id() {
        let declaration = must(
            PluginAgentTrace::search(
                "claude_code",
                "{home}/.claude/projects",
                "**/{agent_session_id}.jsonl",
            ),
            "search declaration",
        );
        let (service, temp, agent) = service_with(declaration);
        let root = temp.path().join("home/.claude/projects");
        let nested = root.join("some-project");
        fs::create_dir_all(&nested).expect("project dir");
        let path = nested.join("abc-123.jsonl");
        fs::write(&path, "{\"type\":\"user\"}\n").expect("write transcript");

        let stat = must_some(service.stat(&agent, "abc-123"), "stat via search");
        assert!(stat.exists);
        let chunk = must_some(service.read(&agent, "abc-123", 0, 1024), "read via search");
        assert_eq!(chunk.text, "{\"type\":\"user\"}\n");

        // A missing session stays missing.
        let missing = must_some(service.stat(&agent, "def-456"), "stat for missing session");
        assert!(!missing.exists);
    }

    #[test]
    fn child_read_requires_the_id_to_appear_in_the_parent() {
        let (service, temp, agent) = service_with(file_declaration());
        let trace_dir = temp.path().join("data/opencode/trace");
        fs::create_dir_all(&trace_dir).expect("trace dir");
        fs::write(
            trace_dir.join("ses_parent.ndjson"),
            "{\"type\":\"subagent.spawn\",\"childSessionID\":\"ses_child\"}\n",
        )
        .expect("parent trace");
        fs::write(
            trace_dir.join("ses_child.ndjson"),
            "{\"type\":\"step.start\"}\n",
        )
        .expect("child trace");

        let chunk = must_some(
            service.read_child(&agent, "ses_parent", "ses_child", 0, 1024),
            "child read",
        );
        assert_eq!(chunk.text, "{\"type\":\"step.start\"}\n");

        // A child id that never appears in the parent is refused, not just missing.
        assert!(
            service
                .read_child(&agent, "ses_parent", "ses_stranger", 0, 1024)
                .is_none()
        );
    }

    #[test]
    fn list_scans_declared_roots_and_extracts_names() {
        let (service, temp, agent) = service_with(file_declaration());
        let trace_dir = temp.path().join("data/opencode/trace");
        fs::create_dir_all(&trace_dir).expect("trace dir");
        fs::write(
            trace_dir.join("ses_a.ndjson"),
            "{\"type\":\"session.start\",\"title\":\"标题A\",\"sessionID\":\"ses_a\"}\n",
        )
        .expect("trace a");
        fs::write(
            trace_dir.join("ses_b.ndjson"),
            "{\"type\":\"session.start\",\"title\":\"标题B\",\"sessionID\":\"ses_b\"}\n",
        )
        .expect("trace b");

        let entries = service.list(Some(&agent));
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ses_a", "ses_b"]
        );
        assert_eq!(entries[0].name.as_deref(), Some("标题A"));

        // Filtering by another agent returns nothing.
        let other = must(AgentRef::parse("ora-space.other"), "other agent ref");
        assert!(service.list(Some(&other)).is_empty());
    }

    #[test]
    fn list_claude_names_fall_back_to_the_cwd_basename() {
        let declaration = must(
            PluginAgentTrace::search(
                "claude_code",
                "{home}/.claude/projects",
                "**/{agent_session_id}.jsonl",
            ),
            "search declaration",
        );
        let (service, temp, agent) = service_with(declaration);
        let root = temp.path().join("home/.claude/projects/my-dashboard");
        fs::create_dir_all(&root).expect("project dir");
        fs::write(
            root.join("abc-123.jsonl"),
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"x\"}]},\"cwd\":\"/home/u/projects/my-dashboard\",\"uuid\":\"a\"}\n",
        )
        .expect("transcript without user text");

        let entries = service.list(Some(&agent));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name.as_deref(), Some("my-dashboard"));
    }

    #[test]
    fn wildcard_matcher_covers_star_and_double_star() {
        assert!(matches_pattern("**/*.jsonl", "a/b/c.jsonl"));
        assert!(matches_pattern("**/*.jsonl", "c.jsonl"));
        assert!(matches_pattern("**/ses_*.ndjson", "x/y/ses_1.ndjson"));
        assert!(matches_pattern("*/*.jsonl", "p/abc.jsonl"));
        assert!(!matches_pattern("**/*.jsonl", "a/b/c.ndjson"));
        assert!(!matches_pattern("*.jsonl", "a/b.jsonl"));
    }

    #[test]
    fn session_ids_are_extracted_from_matched_files() {
        assert_eq!(
            match_session("x/ses_abc.ndjson", "**/*.ndjson"),
            Some(("ses_abc".to_owned(), "ses_abc.ndjson".to_owned()))
        );
        assert_eq!(
            match_session("abc-123.jsonl", "**/*.jsonl"),
            Some(("abc-123".to_owned(), "abc-123.jsonl".to_owned()))
        );
        assert_eq!(match_session("x/a.ndjson", "**/*.jsonl"), None);
    }
}
