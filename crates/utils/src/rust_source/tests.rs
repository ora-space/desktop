use super::{
    RustModuleReference, RustSourceAnalysis, RustSourceKind, RustSourceUse, analyze_rust_source,
};
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};

/// Counts a standalone module without involving any repository policy or filesystem fixture.
fn analyze(source: &str) -> RustSourceAnalysis {
    analyze_rust_source(source, Path::new("src/domain.rs"), RustSourceKind::Module).unwrap()
}

/// Builds expected links with platform-native path components.
fn reference(paths: &[&[&str]], usage: RustSourceUse) -> RustModuleReference {
    let mut candidates: Vec<PathBuf> = paths.iter().map(|parts| parts.iter().collect()).collect();
    candidates.sort();
    RustModuleReference { candidates, usage }
}

/// An early test-only declaration must not hide the following implementation or its comments.
#[test]
fn keeps_production_after_the_first_test_module() {
    let source = "#[cfg(test)]\nmod lifecycle_tests;\n\n/// Owns the operation.\npub struct Service;\n#[cfg(test)]\nmod tests { fn test_helper() {} }\nimpl Service {\n    pub fn execute(&self) {}\n}\n";
    assert_eq!(
        analyze(source),
        RustSourceAnalysis {
            production_lines: 5,
            modules: vec![reference(
                &[
                    &["src", "domain", "lifecycle_tests.rs"],
                    &["src", "domain", "lifecycle_tests", "mod.rs"]
                ],
                RustSourceUse::TestOnly
            )],
        }
    );
}

/// Removes inline tests at any position while retaining a production item sharing the same line.
#[test]
fn preserves_shared_lines_and_unicode() {
    assert_eq!(
        analyze(
            "#[cfg(test)] fn test() {} const 名称: &str = \"你好\";\n#[test]\nfn test_two() {}\n"
        ),
        RustSourceAnalysis {
            production_lines: 1,
            modules: vec![]
        }
    );
}

/// Source offsets remain correct when syn removes a BOM and Unix interpreter directive.
#[test]
fn restores_bom_and_shebang_offsets() {
    assert_eq!(
        analyze(
            "\u{feff}#!/usr/bin/env rust-script\n#[cfg(test)]\nmod tests {}\nfn production() {}\n"
        ),
        RustSourceAnalysis {
            production_lines: 2,
            modules: vec![]
        }
    );
}

/// A crate-wide test cfg excludes its helpers and classifies every child link as test-only.
#[test]
fn handles_inner_file_attributes() {
    assert_eq!(
        analyze("#![cfg(test)]\nuse std::fmt;\nmod fixture;\nfn helper() {}\n"),
        RustSourceAnalysis {
            production_lines: 0,
            modules: vec![reference(
                &[
                    &["src", "domain", "fixture.rs"],
                    &["src", "domain", "fixture", "mod.rs"]
                ],
                RustSourceUse::TestOnly
            )],
        }
    );
}

/// Retains unknown platform/feature branches, including an any(test, feature) production branch.
#[test]
fn evaluates_compound_cfg_without_assuming_host_features() {
    let source = "#[cfg(all(test, unix))]\nfn removed() {}\n#[cfg(any(test, feature = \"extra\"))]\nfn feature_enabled() {}\n#[cfg(not(test))]\nfn production_only() {}\n#[cfg(windows)]\nfn other_platform() {}\n#[cfg_attr(not(test), cfg(test))]\nfn also_removed() {}\n#[cfg_attr(feature = \"maybe\", cfg(test))]\nfn maybe_production() {}\n";
    assert_eq!(
        analyze(source),
        RustSourceAnalysis {
            production_lines: 8,
            modules: vec![]
        }
    );
}

/// Attributes on members and statements remove only those nodes, not the enclosing impl or function.
#[test]
fn excludes_test_members_fields_and_statements() {
    let source = "struct Service {\n#[cfg(test)]\nfixture: u8,\nvalue: u8,\n}\nimpl Service {\n#[cfg(test)]\nfn helper(&self) {}\nfn execute(&self) {\n#[cfg(test)]\nlet local = 1;\n#[cfg(test)]\nassert!(true);\nlet value = 2;\n}\n}\ntrait Contract {\n#[cfg(test)]\nfn helper();\nfn operation();\n}\n";
    assert_eq!(
        analyze(source),
        RustSourceAnalysis {
            production_lines: 11,
            modules: vec![]
        }
    );
}

/// Module filenames use the source stem, while root modules and legacy mod.rs use their directory.
#[test]
fn resolves_root_and_legacy_module_children() {
    let expected = RustSourceAnalysis {
        production_lines: 1,
        modules: vec![
            reference(
                &[&["src", "child.rs"], &["src", "child", "mod.rs"]],
                RustSourceUse::Production,
            ),
            reference(
                &[&["src", "child.rs"], &["src", "child", "mod.rs"]],
                RustSourceUse::TestOnly,
            ),
        ],
    };
    assert_eq!(
        analyze_rust_source(
            "mod child;",
            Path::new("src/custom_entry.rs"),
            RustSourceKind::CrateRoot
        )
        .unwrap(),
        expected
    );
    assert_eq!(
        analyze_rust_source(
            "mod child;",
            Path::new("src/mod.rs"),
            RustSourceKind::Module
        )
        .unwrap(),
        expected
    );
}

/// Explicit module paths are relative to the source directory, including test-only declarations.
#[test]
fn resolves_explicit_paths_and_cfg_attr_test_paths() {
    let source = "#[cfg_attr(test, path = \"test_adapter.rs\")]\n#[cfg_attr(not(test), path = \"adapter.rs\")]\nmod adapter;\n";
    assert_eq!(
        analyze(source),
        RustSourceAnalysis {
            production_lines: 3,
            modules: vec![
                reference(&[&["src", "adapter.rs"]], RustSourceUse::Production),
                reference(&[&["src", "test_adapter.rs"]], RustSourceUse::TestOnly),
            ],
        }
    );
}

/// Inline modules add directory components and explicit inline paths replace that directory.
#[test]
fn resolves_nested_inline_module_paths() {
    let source = "mod nested { #[path = \"adapter.rs\"] mod adapter; }\n#[path = \"fixtures\"] mod tests { #[cfg(test)] mod child; }\n";
    assert_eq!(
        analyze(source),
        RustSourceAnalysis {
            production_lines: 2,
            modules: vec![
                reference(
                    &[&["src", "domain", "nested", "adapter.rs"]],
                    RustSourceUse::Production
                ),
                reference(
                    &[
                        &["src", "fixtures", "child.rs"],
                        &["src", "fixtures", "child", "mod.rs"]
                    ],
                    RustSourceUse::TestOnly
                ),
                reference(
                    &[&["src", "domain", "nested", "adapter.rs"]],
                    RustSourceUse::TestOnly
                ),
            ],
        }
    );
}

/// Unknown cfg_attr paths retain both alternatives instead of selecting the current host platform.
#[test]
fn includes_conditional_module_path_alternatives() {
    let source = "#[cfg_attr(windows, path = \"windows.rs\")]\nmod adapter;";
    let mut candidates = vec![
        Path::new("src").join("domain").join("adapter.rs"),
        Path::new("src")
            .join("domain")
            .join("adapter")
            .join("mod.rs"),
        Path::new("src").join("windows.rs"),
    ];
    candidates.sort();
    assert_eq!(
        analyze(source),
        RustSourceAnalysis {
            production_lines: 2,
            modules: vec![
                RustModuleReference {
                    candidates: candidates.clone(),
                    usage: RustSourceUse::Production
                },
                RustModuleReference {
                    candidates,
                    usage: RustSourceUse::TestOnly
                },
            ],
        }
    );
}

/// Unexpanded macro bodies stay counted; matching cfg text inside comments and strings is inert.
#[test]
fn does_not_strip_text_that_only_looks_like_test_syntax() {
    let source = "// #[cfg(test)]\nconst TEXT: &str = r#\"#[cfg(test)]\"#;\nmacro_rules! generated { () => {\n#[cfg(test)] fn maybe_generated() {}\n}; }\n";
    assert_eq!(
        analyze(source),
        RustSourceAnalysis {
            production_lines: 5,
            modules: vec![]
        }
    );
}

/// Broken Rust and malformed built-in cfg syntax cannot be mistaken for an empty production file.
#[test]
fn rejects_malformed_source_and_cfg() {
    for source in [
        "fn broken(",
        "#[cfg(not(test, unix))] fn invalid() {}",
        "#[path = 42] mod wrong;",
    ] {
        assert!(
            analyze_rust_source(source, Path::new("src/lib.rs"), RustSourceKind::CrateRoot)
                .is_err()
        );
    }
}
