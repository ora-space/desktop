use super::CheckDtArgs;
use super::catalog::{Catalog, CatalogProblem, parse_catalog};
use super::check::{CheckOptions, Code, Diagnostic, check};
use super::declaration::{Declaration, FeatureRef, Kind, ParseError, parse_declaration};
use super::resolve::{Ownership, ScopedFiles, collect_scope, resolve_ownership};
use super::scan::{DeclarationLine, Header, HeaderTarget, scan_headers};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// DT[declaration-grammar][happy] A local feature, a known kind, and a statement parse into the declaration triple.
#[test]
fn parses_local_declaration() {
    assert_eq!(
        parse_declaration("DT[branch-listing][happy] Local refs are listed with display labels."),
        Ok(Declaration {
            feature: FeatureRef::Local("branch-listing".to_string()),
            kind: Kind::Happy,
            statement: "Local refs are listed with display labels.".to_string(),
        })
    );
}

/// DT[declaration-grammar][happy] A `seg::seg::id` feature parses into its module segments and id.
#[test]
fn parses_qualified_declaration() {
    assert_eq!(
        parse_declaration(" DT[parse::commit::readback-id][edge] Empty summaries survive."),
        Ok(Declaration {
            feature: FeatureRef::Qualified {
                segments: vec!["parse".to_string(), "commit".to_string()],
                id: "readback-id".to_string(),
            },
            kind: Kind::Edge,
            statement: "Empty summaries survive.".to_string(),
        })
    );
}

/// DT[declaration-grammar][edge] The reserved word `todo` is accepted in either field and both at once.
#[test]
fn accepts_todo_in_both_fields() {
    assert_eq!(
        parse_declaration("DT[todo][todo] Not yet classified.")
            .map(|declaration| (declaration.feature, declaration.kind)),
        Ok((FeatureRef::Todo, Kind::Todo))
    );
    assert_eq!(
        parse_declaration("DT[todo][error] Rejects garbage.")
            .map(|declaration| declaration.feature),
        Ok(FeatureRef::Todo)
    );
}

/// DT[declaration-grammar][error] Grammar violations report which part is malformed instead of guessing.
#[test]
fn rejects_malformed_declarations() {
    let cases = [
        (
            "DT[branch-listing][happy]",
            "statement must be separated by one space",
        ),
        (
            "DT[branch-listing][happy]  two spaces",
            "statement must not be empty or start with whitespace",
        ),
        (
            "DT[branch-listing][happy] trailing ",
            "statement must not end with whitespace",
        ),
        (
            "DT[branch-listing] [happy] gap",
            "kind field must directly follow the feature field",
        ),
        (
            "DT[Branch-Listing][happy] upper",
            "feature id must be kebab-case: [a-z0-9]+(-[a-z0-9]+)*",
        ),
        (
            "DT[branch--listing][happy] double dash",
            "feature id must be kebab-case: [a-z0-9]+(-[a-z0-9]+)*",
        ),
        (
            "DT[bad-seg::x-y][happy] qualified",
            "module qualifier segments must be Rust identifiers separated by `::`",
        ),
        (
            "DT[parse::todo][happy] qualified todo",
            "`todo` cannot be module-qualified",
        ),
        ("DT[branch-listing][happy", "unterminated kind field"),
    ];
    let actual: Vec<Result<Declaration, ParseError>> = cases
        .iter()
        .map(|(doc, _)| parse_declaration(doc))
        .collect();
    let expected: Vec<Result<Declaration, ParseError>> = cases
        .iter()
        .map(|(_, reason)| Err(ParseError::Malformed(reason)))
        .collect();
    assert_eq!(actual, expected);
}

/// DT[declaration-grammar][error] An unknown kind is reported separately from grammar errors so the message can list the enum.
#[test]
fn rejects_unknown_kind() {
    assert_eq!(
        parse_declaration("DT[branch-listing][regression] Fixed once."),
        Err(ParseError::UnknownKind("regression".to_string()))
    );
}

/// DT[header-scanning][happy] Doc lines, plain comments, and attributes before `fn` form one header whose target knows the test attribute.
#[test]
fn scans_test_function_headers() {
    let source = "\
|mod tests {
|    /// DT[a-b][happy] First.
|    // a plain comment is transparent
|    #[tokio::test(flavor = \"multi_thread\",
|        worker_threads = 4)]
|    async fn first() {}
|
|    #[test]
|    fn missing_declaration() {}
|
|    /// Helper docs.
|    fn helper() {}
|}
";
    assert_eq!(
        scan_headers(&fixture(source)),
        vec![
            Header {
                line: 6,
                declarations: vec![DeclarationLine {
                    line: 2,
                    doc: " DT[a-b][happy] First.".to_string(),
                    is_first_header_line: true,
                }],
                target: HeaderTarget::Function {
                    name: "first".to_string(),
                    is_test: true,
                },
            },
            Header {
                line: 9,
                declarations: vec![],
                target: HeaderTarget::Function {
                    name: "missing_declaration".to_string(),
                    is_test: true,
                },
            },
            Header {
                line: 12,
                declarations: vec![],
                target: HeaderTarget::Function {
                    name: "helper".to_string(),
                    is_test: false,
                },
            },
        ]
    );
}

/// DT[header-scanning][edge] A declaration that is not the first header line, one that is detached by a blank line, and one on a struct are each classified distinctly.
#[test]
fn classifies_misplaced_declarations() {
    let source = "\
|/// Some intro.
|/// DT[a-b][happy] Late.
|#[test]
|fn late() {}
|
|/// DT[a-b][happy] Detached.
|
|#[test]
|fn detached() {}
|
|/// DT[a-b][happy] On a struct.
|struct Fixture;
";
    assert_eq!(
        scan_headers(&fixture(source)),
        vec![
            Header {
                line: 4,
                declarations: vec![DeclarationLine {
                    line: 2,
                    doc: " DT[a-b][happy] Late.".to_string(),
                    is_first_header_line: false,
                }],
                target: HeaderTarget::Function {
                    name: "late".to_string(),
                    is_test: true,
                },
            },
            Header {
                line: 7,
                declarations: vec![DeclarationLine {
                    line: 6,
                    doc: " DT[a-b][happy] Detached.".to_string(),
                    is_first_header_line: true,
                }],
                target: HeaderTarget::Detached,
            },
            Header {
                line: 9,
                declarations: vec![],
                target: HeaderTarget::Function {
                    name: "detached".to_string(),
                    is_test: true,
                },
            },
            Header {
                line: 12,
                declarations: vec![DeclarationLine {
                    line: 11,
                    doc: " DT[a-b][happy] On a struct.".to_string(),
                    is_first_header_line: true,
                }],
                target: HeaderTarget::OtherItem,
            },
        ]
    );
}

/// DT[catalog-parsing][happy] Entries are collected from the section while prose, sub-headings, and fenced code are ignored.
#[test]
fn parses_catalog_section() {
    let markdown = "\
|# Module
|
|## Guarantees
|
|- `not-an-entry`: outside the section
|
|## Feature points
|
|Stable identifiers.
|
|### Reads
|
|- `file-read`: Bounded UTF-8 reads.
|  Continuation line is ignored.
|- `dir-listing`: Sorted listings.
|
|```markdown
|- `fenced`: never parsed
|```
|
|## Next section
|
|- `after`: also outside
";
    assert_eq!(
        parse_catalog(&fixture(markdown)),
        Catalog {
            has_section: true,
            entries: [
                ("dir-listing".to_string(), "Sorted listings.".to_string()),
                ("file-read".to_string(), "Bounded UTF-8 reads.".to_string()),
            ]
            .into_iter()
            .collect(),
            problems: vec![],
        }
    );
}

/// DT[catalog-parsing][error] Malformed entries, duplicates, and the reserved word are reported with their line numbers.
#[test]
fn reports_catalog_problems() {
    let markdown = "\
|## Feature points
|
|- `file-read`: Reads.
|- `file-read`: Again.
|- `todo`: Reserved.
|- plain bullet
|- `Bad_Id`: description
|- `no-desc`:
|- `no-colon` description
";
    let catalog = parse_catalog(&fixture(markdown));
    assert_eq!(
        catalog.entries.keys().collect::<Vec<_>>(),
        vec!["file-read"]
    );
    assert_eq!(
        catalog.problems,
        vec![
            CatalogProblem {
                line: 4,
                message: "duplicate feature point `file-read`".to_string()
            },
            CatalogProblem {
                line: 5,
                message: "`todo` is a reserved word and cannot be a feature point".to_string()
            },
            CatalogProblem {
                line: 6,
                message:
                    "feature point entries must start with a backticked id: - `id`: description"
                        .to_string()
            },
            CatalogProblem {
                line: 7,
                message: "feature id must be kebab-case: [a-z0-9]+(-[a-z0-9]+)*".to_string()
            },
            CatalogProblem {
                line: 8,
                message: "expected a non-empty description after `: `".to_string()
            },
            CatalogProblem {
                line: 9,
                message: "expected `:` directly after the backticked id".to_string()
            },
        ]
    );
}

/// DT[catalog-parsing][edge] A README without the section yields an empty catalog flagged as sectionless rather than an error.
#[test]
fn distinguishes_missing_section() {
    assert_eq!(
        parse_catalog("# Only prose\n\n- `x`: y\n"),
        Catalog::default()
    );
}

/// DT[ownership-resolution][happy] The nearest README between the file and the crate root owns the file; deeper files skip READMEs above them only when a closer one exists.
#[test]
fn resolves_nearest_readme_within_crate() {
    let workspace = FixtureCrate::new();
    workspace.write("README.md", "");
    workspace.write("src/spec/README.md", "");
    let nested = workspace.write("src/spec/inner/leaf.rs", "");
    let root_level = workspace.write("src/lib.rs", "");
    let integration = workspace.write("tests/api.rs", "");

    assert_eq!(
        (
            resolve_ownership(&nested),
            resolve_ownership(&root_level),
            resolve_ownership(&integration)
        ),
        (
            Ok(Ownership {
                crate_root: workspace.root(),
                owning_readme: Some(workspace.path("src/spec/README.md"))
            }),
            Ok(Ownership {
                crate_root: workspace.root(),
                owning_readme: Some(workspace.path("README.md"))
            }),
            Ok(Ownership {
                crate_root: workspace.root(),
                owning_readme: Some(workspace.path("README.md"))
            }),
        )
    );
}

/// DT[ownership-resolution][edge] A crate without any README resolves to its root with no owning README instead of climbing past `Cargo.toml`.
#[test]
fn stops_at_crate_root_without_readme() {
    let workspace = FixtureCrate::new();
    fs::write(workspace.temp.path().join("README.md"), "").expect("write parent README");
    let file = workspace.write("src/lib.rs", "");
    assert_eq!(
        resolve_ownership(&file),
        Ok(Ownership {
            crate_root: workspace.root(),
            owning_readme: None
        })
    );
}

/// DT[ownership-resolution][happy] Scope collection buckets Rust files and READMEs and skips build directories.
#[test]
fn collects_scope_files() {
    let workspace = FixtureCrate::new();
    workspace.write("README.md", "");
    workspace.write("src/a.rs", "");
    workspace.write("target/debug/generated.rs", "");
    workspace.write("src/notes.txt", "");
    assert_eq!(
        collect_scope(&[workspace.root()]),
        Ok(ScopedFiles {
            rust_files: vec![workspace.path("src/a.rs")],
            readmes: vec![workspace.path("README.md")],
        })
    );
}

/// DT[rule-evaluation][happy] A crate whose tests all reference catalogued features passes with counts and no diagnostics.
#[test]
fn accepts_conforming_crate() {
    let workspace = FixtureCrate::new();
    workspace.write("README.md", "## Feature points\n\n- `root-thing`: Root.\n");
    workspace.write(
        "src/spec/README.md",
        "## Feature points\n\n- `spec-read`: Reads specs.\n",
    );
    workspace.write(
        "src/spec/mod.rs",
        "/// DT[spec-read][happy] Reads.\n#[test]\nfn reads() {}\n\n/// DT[spec-read][todo] Unsure.\n#[test]\nfn unsure() {}\n",
    );
    workspace.write(
        "tests/api.rs",
        "/// DT[root-thing][error] Fails.\n#[test]\nfn fails() {}\n\n/// DT[spec::spec-read][edge] Qualified.\n#[test]\nfn qualified() {}\n",
    );

    let report = check(&workspace.scope(), CheckOptions::default()).expect("check runs");
    assert_eq!(
        (
            report.diagnostics,
            report.tests_checked,
            report.files_with_tests,
            report.todo_declarations,
            report.todo_features,
            report.todo_kinds
        ),
        (vec![], 4, 2, 1, 0, 1)
    );
}

/// DT[rule-evaluation][error] Each rule fires on its own trigger and diagnostics are sorted by path and line.
#[test]
fn reports_each_rule() {
    let workspace = FixtureCrate::new();
    workspace.write(
        "README.md",
        "## Feature points\n\n- `root-thing`: Root.\n- `root-thing`: Dup.\n",
    );
    workspace.write("src/spec/README.md", "# No section\n");
    workspace.write(
        "src/lib.rs",
        &fixture(
            "\
|#[test]
|fn no_declaration() {}
|
|/// DT[unknown-thing][happy] Unknown feature.
|#[test]
|fn unknown_feature() {}
|
|/// DT[root-thing][weird] Bad kind.
|#[test]
|fn bad_kind() {}
|
|/// DT[root-thing][happy]
|#[test]
|fn malformed() {}
|
|/// DT[root-thing][happy] One.
|/// DT[root-thing][edge] Two.
|#[test]
|fn doubled() {}
|
|/// Intro first.
|/// DT[root-thing][happy] Late.
|#[test]
|fn late() {}
|
|/// DT[root-thing][happy] Helper.
|fn helper() {}
|
|/// DT[spec::spec-read][happy] Target lacks section.
|#[test]
|fn sectionless_target() {}
|
|/// DT[missing::thing][happy] Target missing.
|#[test]
|fn missing_target() {}
",
        ),
    );
    workspace.write(
        "src/spec/mod.rs",
        "/// DT[spec::spec-read][happy] Redundant qualifier.\n#[test]\nfn redundant() {}\n\n/// DT[spec-read][happy] Sectionless owner.\n#[test]\nfn owner_lacks_section() {}\n",
    );

    let report = check(&workspace.scope(), CheckOptions::default()).expect("check runs");
    let codes: Vec<(PathBuf, usize, Code)> = report
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.path.clone(), diagnostic.line, diagnostic.code))
        .collect();
    assert_eq!(
        codes,
        vec![
            (workspace.path("README.md"), 4, Code::Dt009),
            (workspace.path("src/lib.rs"), 2, Code::Dt001),
            (workspace.path("src/lib.rs"), 6, Code::Dt004),
            (workspace.path("src/lib.rs"), 8, Code::Dt003),
            (workspace.path("src/lib.rs"), 12, Code::Dt002),
            (workspace.path("src/lib.rs"), 17, Code::Dt006),
            (workspace.path("src/lib.rs"), 22, Code::Dt007),
            (workspace.path("src/lib.rs"), 26, Code::Dt006),
            (workspace.path("src/lib.rs"), 31, Code::Dt005),
            (workspace.path("src/lib.rs"), 35, Code::Dt008),
            (workspace.path("src/spec/mod.rs"), 3, Code::Dt008),
            (workspace.path("src/spec/mod.rs"), 7, Code::Dt005),
        ]
    );
}

/// DT[rule-evaluation][error] With `deny_todo`, every declaration using the reserved word becomes a DT010 violation.
#[test]
fn denies_todo_when_requested() {
    let workspace = FixtureCrate::new();
    workspace.write("README.md", "## Feature points\n\n- `root-thing`: Root.\n");
    workspace.write(
        "src/lib.rs",
        "/// DT[todo][happy] A.\n#[test]\nfn a() {}\n\n/// DT[root-thing][todo] B.\n#[test]\nfn b() {}\n\n/// DT[root-thing][happy] C.\n#[test]\nfn c() {}\n",
    );

    let report = check(&workspace.scope(), CheckOptions { deny_todo: true }).expect("check runs");
    assert_eq!(
        report.diagnostics,
        vec![
            Diagnostic {
                path: workspace.path("src/lib.rs"),
                line: 3,
                code: Code::Dt010,
                message: "test `a` uses the reserved word `todo` while --deny-todo is active"
                    .to_string(),
            },
            Diagnostic {
                path: workspace.path("src/lib.rs"),
                line: 7,
                code: Code::Dt010,
                message: "test `b` uses the reserved word `todo` while --deny-todo is active"
                    .to_string(),
            },
        ]
    );
}

/// DT[rule-evaluation][error] A crate with tests but no README at all is reported once per file as DT005.
#[test]
fn reports_missing_crate_readme_once_per_file() {
    let workspace = FixtureCrate::new();
    workspace.write(
        "src/lib.rs",
        "/// DT[x-y][happy] A.\n#[test]\nfn a() {}\n\n/// DT[x-y][happy] B.\n#[test]\nfn b() {}\n",
    );
    let report = check(&workspace.scope(), CheckOptions::default()).expect("check runs");
    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.line, diagnostic.code))
            .collect::<Vec<_>>(),
        vec![(3, Code::Dt005)]
    );
}

/// DT[command-line][happy] Positional paths and the `--deny-todo` flag parse in any order.
#[test]
fn parses_arguments() {
    assert_eq!(
        CheckDtArgs::parse([
            "crates/fs".to_string(),
            "--deny-todo".to_string(),
            "xtask".to_string()
        ]),
        Ok(CheckDtArgs {
            paths: vec![PathBuf::from("crates/fs"), PathBuf::from("xtask")],
            deny_todo: true,
        })
    );
}

/// DT[command-line][error] An unknown flag is rejected with the usage line.
#[test]
fn rejects_unknown_flags() {
    assert_eq!(
        CheckDtArgs::parse(["--strict".to_string()]),
        Err(
            "unknown option `--strict`; usage: cargo xtask check-dt [--deny-todo] [PATH...]"
                .to_string()
        )
    );
}

/// Temporary crate layout with a `Cargo.toml` at its root.
struct FixtureCrate {
    temp: TempDir,
}

impl FixtureCrate {
    fn new() -> Self {
        let temp = TempDir::new().expect("create temp dir");
        let root = temp.path().join("crate");
        fs::create_dir_all(&root).expect("create crate root");
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
            .expect("write Cargo.toml");
        Self { temp }
    }

    fn root(&self) -> PathBuf {
        self.temp.path().join("crate")
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root().join(relative)
    }

    /// Writes a file inside the crate, creating parent directories, and returns its path.
    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, content).expect("write fixture");
        path
    }

    fn scope(&self) -> ScopedFiles {
        collect_scope(&[self.root()]).expect("collect scope")
    }
}

/// Strips the `|` margin used so fixture text is not mistaken for real declarations by `check-dt`.
fn fixture(text: &str) -> String {
    let mut output: String = text
        .lines()
        .map(|line| line.strip_prefix('|').unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    output.push('\n');
    output
}
