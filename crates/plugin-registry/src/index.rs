use std::fs;
use std::path::{Path, PathBuf};

use ora_logging::ora_warn;
use ora_plugin_manifest::PluginManifest;

use crate::entry::{RegistryEntry, entry_id};
use crate::error::RegistryError;
use crate::logo;

/// The index schema version reported in every built index file.
const INDEX_VERSION: &str = "1.0";

/// Holds one immutable registry index that lists every discoverable marketplace plugin.
///
/// The index is a lightweight derived artifact: callers read it instead of re-scanning and
/// parsing every `orax.toml`, so `updated_at` and the schema `version` are stable pointers
/// that help consumers detect staleness and schema changes.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RegistryIndex {
    updated_at: i64,
    version: String,
    plugins: Vec<RegistryEntry>,
}

impl RegistryIndex {
    /// Scans `dir` recursively for `orax.toml` files, parses each valid manifest, and returns a
    /// deterministically ordered index built at the injected Unix `updated_at` instant.
    ///
    /// Malformed or unreadable manifests are skipped, logged as warnings, and reported through
    /// the returned [`RegistryBuild`] so a single bad file never blocks the whole build.
    pub fn build(dir: &Path, updated_at: i64) -> RegistryBuild {
        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        for path in orax_manifest_paths(dir) {
            match parse_manifest(&path) {
                Ok(manifest) => {
                    let logo = logo::read_beside_manifest(&path);
                    entries.push(RegistryEntry::from_manifest(&manifest, logo));
                }
                Err(error) => {
                    ora_warn!(path = %path.display(), %error, "skipping invalid registry plugin manifest");
                    skipped.push(SkippedManifest {
                        path,
                        reason: error.to_string(),
                    });
                }
            }
        }
        entries.sort_by(|left, right| left.id().cmp(right.id()));

        let index = Self {
            updated_at,
            version: INDEX_VERSION.to_owned(),
            plugins: entries,
        };
        RegistryBuild { index, skipped }
    }

    /// Resolves the full release manifest for a marketplace identifier by re-reading the source
    /// `registry` directory, matching `namespace/name` against each parsed manifest.
    ///
    /// This is the install-time companion of [`Self::build`]: the cached index carries only the
    /// lightweight display fields, so consumers re-read the source `orax.toml` to obtain the
    /// release `url` and `sha256` needed to download and verify. Unparseable manifests are skipped
    /// here exactly as they are during the index build, so one bad file never blocks a lookup.
    pub fn resolve_manifest(
        registry_dir: &Path,
        id: &str,
    ) -> Result<Option<PluginManifest>, RegistryError> {
        for path in orax_manifest_paths(registry_dir) {
            let manifest = match parse_manifest(&path) {
                Ok(manifest) => manifest,
                Err(error) => {
                    ora_warn!(path = %path.display(), %error, "skipping invalid registry plugin manifest while resolving");
                    continue;
                }
            };
            if entry_id(&manifest) == id {
                return Ok(Some(manifest));
            }
        }
        Ok(None)
    }
    /// Loads an index from a previously written JSON file so consumers can read it without rescanning.
    pub fn load(path: &Path) -> Result<Self, RegistryError> {
        let bytes = fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Atomically replaces `path` with this index's JSON serialization through a same-directory
    /// temporary file, so concurrent readers never observe a partially written index.
    pub fn write(&self, path: &Path) -> Result<(), RegistryError> {
        let bytes = serde_json::to_vec(self)?;
        ora_utils::atomic::write(path, &bytes)?;
        Ok(())
    }

    /// Returns the Unix timestamp (seconds) at which this index was built.
    pub fn updated_at(&self) -> i64 {
        self.updated_at
    }

    /// Returns the schema version this index conforms to.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the listed plugins.
    pub fn plugins(&self) -> &[RegistryEntry] {
        &self.plugins
    }
}

/// Holds one built index together with every manifest that was skipped during the scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryBuild {
    index: RegistryIndex,
    skipped: Vec<SkippedManifest>,
}

impl RegistryBuild {
    /// Returns the completed, deterministically ordered index.
    pub fn index(&self) -> &RegistryIndex {
        &self.index
    }

    /// Returns the manifests that were skipped during the build, in path order.
    pub fn skipped(&self) -> &[SkippedManifest] {
        &self.skipped
    }
}

/// Describes one `orax.toml` that could not be parsed into the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedManifest {
    path: PathBuf,
    reason: String,
}

impl SkippedManifest {
    /// Returns the path of the manifest that was skipped.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the human-readable reason the manifest was skipped.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Collects every `orax.toml` beneath `root` in deterministic path order.
fn orax_manifest_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_orax_manifests(root, &mut paths);
    paths.sort();
    paths
}

/// Recursively accumulates `orax.toml` paths without following symlinks or reporting missing roots.
fn collect_orax_manifests(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_orax_manifests(&path, paths);
        } else if path
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new("orax.toml"))
        {
            paths.push(path);
        }
    }
}

/// Reads and parses one manifest, mapping both I/O and semantic failures onto [`RegistryError`].
fn parse_manifest(path: &Path) -> Result<PluginManifest, RegistryError> {
    let source = fs::read_to_string(path)?;
    Ok(PluginManifest::parse(&source)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    const UPDATED_AT: i64 = 1_776_244_428;

    /// Builds a syntactically valid `orax.toml` string for a named plugin.
    fn valid_manifest(name: &str, description: &str) -> String {
        format!(
            "resolver = 1\n\
             name = \"{name}\"\n\
             namespace = \"official\"\n\
             kind = \"workbench\"\n\
             version = \"1.2.0\"\n\
             description = \"{description}\"\n\
             homepage = \"https://example.com\"\n\
             license = \"MIT\"\n\
             url = \"https://example.com/{name}.orax\"\n\
             sha256 = \"{}\"\n",
            "ab".repeat(32)
        )
    }

    /// Writes a manifest at a nesting level that mimics the two-tier marketplace layout.
    fn write_manifest(root: &Path, name: &str, source: &str) -> Result<PathBuf, std::io::Error> {
        let path = root
            .join("registry")
            .join(&name[..1])
            .join(name)
            .join("orax.toml");
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?;
        fs::create_dir_all(parent)?;
        fs::write(&path, source)?;
        Ok(path)
    }

    /// Verifies entries are ordered by their `namespace/name` identifier regardless of scan order.
    #[test]
    fn builds_deterministically_ordered_index() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        write_manifest(root.path(), "z", &valid_manifest("z", "Z plugin"))?;
        write_manifest(root.path(), "a", &valid_manifest("a", "A plugin"))?;

        let build = RegistryIndex::build(root.path(), UPDATED_AT);

        let a_manifest = PluginManifest::parse(&valid_manifest("a", "A plugin"))?;
        let z_manifest = PluginManifest::parse(&valid_manifest("z", "Z plugin"))?;
        let expected_plugins = vec![
            RegistryEntry::from_manifest(&a_manifest, /*logo*/ None),
            RegistryEntry::from_manifest(&z_manifest, /*logo*/ None),
        ];

        assert_eq!(build.index().plugins().to_vec(), expected_plugins);
        assert_eq!(build.index().updated_at(), UPDATED_AT);
        assert_eq!(build.index().version(), INDEX_VERSION);
        assert_eq!(build.skipped().len(), 0);
        Ok(())
    }

    /// Verifies install-time resolution re-reads a manifest from the source registry by its id.
    #[test]
    fn resolves_manifest_by_identifier() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        write_manifest(
            root.path(),
            "weather",
            &valid_manifest("weather", "Weather plugin"),
        )?;

        let registry_dir = root.path().join("registry");
        let manifest = RegistryIndex::resolve_manifest(&registry_dir, "official/weather")?
            .ok_or_else(|| std::io::Error::other("expected a resolved manifest"))?;

        assert_eq!(manifest.name().as_str(), "weather");
        assert_eq!(manifest.namespace().as_str(), "official");

        let missing = RegistryIndex::resolve_manifest(&registry_dir, "official/absent")?;
        assert!(missing.is_none());
        Ok(())
    }
    /// Verifies the `logo.svg` beside a manifest is inlined into that entry's index record.
    #[test]
    fn inlines_the_logo_beside_each_manifest() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let logo = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="8"/></svg>"#;
        let manifest_path = write_manifest(root.path(), "a", &valid_manifest("a", "A plugin"))?;
        let entry_dir = manifest_path
            .parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?;
        fs::write(entry_dir.join("logo.svg"), logo)?;
        write_manifest(root.path(), "b", &valid_manifest("b", "B plugin"))?;

        let build = RegistryIndex::build(root.path(), UPDATED_AT);

        assert_eq!(build.index().plugins()[0].logo(), Some(logo));
        assert_eq!(build.index().plugins()[1].logo(), None);
        Ok(())
    }

    /// Verifies an unsafe logo is dropped while its plugin still reaches the marketplace listing.
    #[test]
    fn indexes_a_plugin_whose_logo_is_unsafe() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let manifest_path = write_manifest(root.path(), "a", &valid_manifest("a", "A plugin"))?;
        let entry_dir = manifest_path
            .parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?;
        fs::write(
            entry_dir.join("logo.svg"),
            "<svg><script>evil()</script></svg>",
        )?;

        let build =
            ora_logging::with_trace_logging(|| RegistryIndex::build(root.path(), UPDATED_AT));

        assert_eq!(build.index().plugins().len(), 1);
        assert_eq!(build.index().plugins()[0].logo(), None);
        assert_eq!(build.skipped().len(), 0);
        Ok(())
    }

    /// Verifies a missing or empty marketplace registry directory builds a valid empty index.
    #[test]
    fn builds_an_empty_index_for_a_missing_registry_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;

        let build = RegistryIndex::build(&root.path().join("registry"), UPDATED_AT);

        assert_eq!(build.index().plugins().len(), 0);
        assert_eq!(build.skipped().len(), 0);
        Ok(())
    }

    /// Verifies a malformed manifest is skipped, logged, and reported without blocking the build.
    #[test]
    fn skips_invalid_manifest_and_reports_it() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        write_manifest(root.path(), "good", &valid_manifest("good", "Good plugin"))?;
        let bad_path = write_manifest(root.path(), "bad", "this is not valid toml")?;

        let build =
            ora_logging::with_trace_logging(|| RegistryIndex::build(root.path(), UPDATED_AT));

        assert_eq!(build.index().plugins().len(), 1);
        assert_eq!(build.index().plugins()[0].id(), "official/good");
        assert_eq!(build.skipped().len(), 1);
        assert_eq!(build.skipped()[0].path(), bad_path);
        assert!(!build.skipped()[0].reason().is_empty());
        Ok(())
    }

    /// Verifies a written index loads back into an equal in-memory value.
    #[test]
    fn load_round_trips_written_index() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        write_manifest(root.path(), "a", &valid_manifest("a", "A plugin"))?;

        let index = RegistryIndex::build(root.path(), UPDATED_AT)
            .index()
            .clone();
        let target = root.path().join("cache").join("registry_index.json");
        let parent = target
            .parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?;
        fs::create_dir_all(parent)?;
        index.write(&target)?;

        let loaded = RegistryIndex::load(&target)?;
        assert_eq!(loaded, index);
        Ok(())
    }

    /// Verifies write replaces prior content and leaves no same-directory temporary files behind.
    #[test]
    fn write_overwrites_atomically_without_leftovers() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        write_manifest(root.path(), "a", &valid_manifest("a", "A plugin"))?;

        let target = root.path().join("registry_index.json");
        RegistryIndex::build(root.path(), UPDATED_AT)
            .index()
            .write(&target)?;
        let first = fs::read_to_string(&target)?;

        let second_index = RegistryIndex::build(root.path(), UPDATED_AT + 1)
            .index()
            .clone();
        second_index.write(&target)?;
        let second = fs::read_to_string(&target)?;

        assert_ne!(first, second);
        assert_eq!(
            serde_json::from_str::<RegistryIndex>(&second)?,
            second_index
        );
        let leftover_temps = fs::read_dir(root.path())?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(leftover_temps, 0);
        Ok(())
    }

    /// Verifies loading a missing file surfaces an error instead of an empty index.
    #[test]
    fn load_missing_file_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;

        assert!(RegistryIndex::load(&root.path().join("missing.json")).is_err());
        Ok(())
    }
}
