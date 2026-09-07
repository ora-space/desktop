use super::{Source, account};
use crate::rust_architecture::ModuleSize;
use ora_utils::path::normalize_relative;
use ora_utils::rust_source::{RustSourceKind, RustSourceUse, analyze_rust_source};
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Runs the real source parser and graph accounting over a complete in-memory file tree.
fn inspect(
    files: &[(&str, &str)],
    roots: &[(&str, RustSourceUse)],
) -> BTreeMap<PathBuf, ModuleSize> {
    let workspace = Path::new("workspace");
    let roots: BTreeMap<_, _> = roots
        .iter()
        .map(|(path, usage)| (workspace.join(path), *usage))
        .collect();
    let mut sources: BTreeMap<_, _> = files
        .iter()
        .map(|(path, source)| {
            let path = workspace.join(path);
            let kind = if roots.contains_key(&path) {
                RustSourceKind::CrateRoot
            } else {
                RustSourceKind::Module
            };
            let analysis = analyze_rust_source(source, &path, kind).unwrap();
            (
                path,
                Source {
                    owner: "ora-fixture".into(),
                    analysis,
                    links: Vec::new(),
                },
            )
        })
        .collect();
    let paths: Vec<_> = sources.keys().cloned().collect();
    for source in sources.values_mut() {
        for reference in &source.analysis.modules {
            source.links.extend(
                reference
                    .candidates
                    .iter()
                    .map(|path| normalize_relative(path).unwrap())
                    .filter(|path| paths.contains(path))
                    .map(|path| (path, reference.usage)),
            );
        }
    }
    account(workspace, sources, roots).unwrap()
}

/// Builds full expected reports, including which files are excluded rather than just their sizes.
fn expected(files: &[(&str, usize, RustSourceUse)]) -> BTreeMap<PathBuf, ModuleSize> {
    files
        .iter()
        .map(|(path, lines, usage)| {
            (
                PathBuf::from(path),
                ModuleSize {
                    owner: "ora-fixture".into(),
                    production_lines: *lines,
                    test_only: *usage == RustSourceUse::TestOnly,
                },
            )
        })
        .collect()
}

/// Outlined tests and their helpers are excluded even when their declarations precede production.
#[test]
fn follows_test_only_descendants_and_later_production() {
    let report = inspect(
        &[
            ("src/lib.rs", "mod task;"),
            (
                "src/task.rs",
                "#[cfg(test)] mod lifecycle_tests;\npub fn execute() {}",
            ),
            (
                "src/task/lifecycle_tests.rs",
                "mod fixtures;\nfn integration_test() {}",
            ),
            (
                "src/task/lifecycle_tests/fixtures.rs",
                "fn seed_database() {}",
            ),
        ],
        &[("src/lib.rs", RustSourceUse::Production)],
    );
    assert_eq!(
        report,
        expected(&[
            ("src/lib.rs", 1, RustSourceUse::Production),
            ("src/task.rs", 1, RustSourceUse::Production),
            ("src/task/lifecycle_tests.rs", 0, RustSourceUse::TestOnly),
            (
                "src/task/lifecycle_tests/fixtures.rs",
                0,
                RustSourceUse::TestOnly
            ),
        ])
    );
}

/// Cargo integration targets cannot be accidentally promoted by the unreferenced-file fallback.
#[test]
fn preserves_integration_target_classification() {
    let report = inspect(
        &[
            ("src/lib.rs", "pub fn run() {}"),
            ("tests/integration.rs", "mod fixture;\nfn test() {}"),
            ("tests/fixture.rs", "fn setup() {}"),
        ],
        &[
            ("src/lib.rs", RustSourceUse::Production),
            ("tests/integration.rs", RustSourceUse::TestOnly),
        ],
    );
    assert_eq!(
        report,
        expected(&[
            ("src/lib.rs", 1, RustSourceUse::Production),
            ("tests/integration.rs", 0, RustSourceUse::TestOnly),
            ("tests/fixture.rs", 0, RustSourceUse::TestOnly),
        ])
    );
}

/// A file referenced by a real application cannot hide behind another test-only reference.
#[test]
fn production_wins_for_shared_source() {
    let report = inspect(
        &[
            ("src/lib.rs", "#[path = \"shared.rs\"] mod shared;"),
            ("src/shared.rs", "pub fn shared() {}"),
            (
                "tests/integration.rs",
                "#[path = \"../src/shared.rs\"] mod shared;",
            ),
        ],
        &[
            ("src/lib.rs", RustSourceUse::Production),
            ("tests/integration.rs", RustSourceUse::TestOnly),
        ],
    );
    assert_eq!(
        report,
        expected(&[
            ("src/lib.rs", 1, RustSourceUse::Production),
            ("src/shared.rs", 1, RustSourceUse::Production),
            ("tests/integration.rs", 0, RustSourceUse::TestOnly),
        ])
    );
}

/// Unreferenced implementation is counted while its explicitly test-only descendants are not.
#[test]
fn counts_orphan_source_without_promoting_its_tests() {
    let report = inspect(
        &[
            ("src/lib.rs", "pub fn run() {}"),
            ("src/orphan.rs", "#[cfg(test)] mod tests;\nfn retained() {}"),
            ("src/orphan/tests.rs", "fn helper() {}"),
        ],
        &[("src/lib.rs", RustSourceUse::Production)],
    );
    assert_eq!(
        report,
        expected(&[
            ("src/lib.rs", 1, RustSourceUse::Production),
            ("src/orphan.rs", 1, RustSourceUse::Production),
            ("src/orphan/tests.rs", 0, RustSourceUse::TestOnly),
        ])
    );
}

/// A cfg_attr(test, path) alternate source is excluded without excluding the production adapter.
#[test]
fn separates_test_and_production_path_alternatives() {
    let report = inspect(
        &[
            (
                "src/lib.rs",
                "#[cfg_attr(test, path = \"test_adapter.rs\")] mod adapter;",
            ),
            ("src/adapter.rs", "pub fn execute() {}"),
            (
                "src/test_adapter.rs",
                "pub fn execute() {}\nfn fixture() {}",
            ),
        ],
        &[("src/lib.rs", RustSourceUse::Production)],
    );
    assert_eq!(
        report,
        expected(&[
            ("src/lib.rs", 1, RustSourceUse::Production),
            ("src/adapter.rs", 1, RustSourceUse::Production),
            ("src/test_adapter.rs", 0, RustSourceUse::TestOnly),
        ])
    );
}
