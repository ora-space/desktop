//! Rule evaluation: turns scanned headers and README catalogs into diagnostics.

use super::catalog::{Catalog, SECTION_HEADING, parse_catalog};
use super::declaration::{Declaration, FeatureRef, Kind, ParseError, parse_declaration};
use super::resolve::{ScopedFiles, qualified_readme, resolve_ownership};
use super::scan::{Header, HeaderTarget, scan_headers};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Stable rule identifiers; the numeric suffix is what appears in output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Code {
    /// Test function without a declaration.
    Dt001,
    /// Declaration attempt that does not match the grammar.
    Dt002,
    /// Unknown kind.
    Dt003,
    /// Feature point not declared in the resolved catalog.
    Dt004,
    /// Owning README missing or without a `## Feature points` section.
    Dt005,
    /// Multiple declarations, or a declaration on a non-test item.
    Dt006,
    /// Declaration is not the first header line, or is detached from its function.
    Dt007,
    /// Redundant or unresolvable module qualifier.
    Dt008,
    /// Malformed catalog section.
    Dt009,
    /// `todo` used while `--deny-todo` is active.
    Dt010,
}

impl fmt::Display for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Dt001 => "DT001",
            Self::Dt002 => "DT002",
            Self::Dt003 => "DT003",
            Self::Dt004 => "DT004",
            Self::Dt005 => "DT005",
            Self::Dt006 => "DT006",
            Self::Dt007 => "DT007",
            Self::Dt008 => "DT008",
            Self::Dt009 => "DT009",
            Self::Dt010 => "DT010",
        };
        formatter.write_str(text)
    }
}

/// One reported violation, anchored to a file line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Diagnostic {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) code: Code,
    pub(crate) message: String,
}

/// Knobs that change which rules fire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CheckOptions {
    pub(crate) deny_todo: bool,
}

/// Aggregate result of one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CheckReport {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) tests_checked: usize,
    pub(crate) files_with_tests: usize,
    /// Declarations that use `todo` in at least one field.
    pub(crate) todo_declarations: usize,
    pub(crate) todo_features: usize,
    pub(crate) todo_kinds: usize,
}

/// Runs every rule over the scoped files.
pub(crate) fn check(files: &ScopedFiles, options: CheckOptions) -> Result<CheckReport, String> {
    let mut checker = Checker {
        options,
        report: CheckReport::default(),
        catalogs: BTreeMap::new(),
        referenced_readmes: BTreeSet::new(),
    };
    for rust_file in &files.rust_files {
        checker.check_rust_file(rust_file)?;
    }
    // Catalog format is checked for READMEs inside the scope and for every README a
    // declaration resolved against, so an out-of-scope but referenced catalog cannot rot.
    let mut readmes: BTreeSet<PathBuf> = files.readmes.iter().cloned().collect();
    readmes.extend(checker.referenced_readmes.iter().cloned());
    for readme in readmes {
        let catalog = checker.catalog(&readme)?.clone();
        for problem in catalog.problems {
            checker.report.diagnostics.push(Diagnostic {
                path: readme.clone(),
                line: problem.line,
                code: Code::Dt009,
                message: problem.message,
            });
        }
    }
    checker.report.diagnostics.sort();
    checker.report.diagnostics.dedup();
    Ok(checker.report)
}

/// Mutable state shared across files during one run.
struct Checker {
    options: CheckOptions,
    report: CheckReport,
    catalogs: BTreeMap<PathBuf, Catalog>,
    referenced_readmes: BTreeSet<PathBuf>,
}

impl Checker {
    /// Loads and caches a README catalog; a missing file yields an empty catalog.
    fn catalog(&mut self, readme: &Path) -> Result<&Catalog, String> {
        if !self.catalogs.contains_key(readme) {
            let markdown = fs::read_to_string(readme)
                .map_err(|error| format!("failed to read {}: {error}", readme.display()))?;
            self.catalogs
                .insert(readme.to_path_buf(), parse_catalog(&markdown));
        }
        Ok(&self.catalogs[readme])
    }

    fn push(&mut self, path: &Path, line: usize, code: Code, message: String) {
        self.report.diagnostics.push(Diagnostic {
            path: path.to_path_buf(),
            line,
            code,
            message,
        });
    }

    /// Applies the per-function rules to one Rust file.
    fn check_rust_file(&mut self, rust_file: &Path) -> Result<(), String> {
        let source = fs::read_to_string(rust_file)
            .map_err(|error| format!("failed to read {}: {error}", rust_file.display()))?;
        let headers = scan_headers(&source);
        let ownership = resolve_ownership(rust_file)?;
        // DT005 is a per-file condition; reporting it on every test would only add noise.
        let mut reported_missing_catalog = false;
        let mut has_tests = false;

        for header in headers {
            match &header.target {
                HeaderTarget::Function {
                    name,
                    is_test: true,
                } => {
                    has_tests = true;
                    self.report.tests_checked += 1;
                    let Some(declaration) = self.declaration_of(rust_file, &header, name) else {
                        continue;
                    };
                    self.check_feature(
                        rust_file,
                        header.line,
                        &declaration,
                        &ownership.crate_root,
                        ownership.owning_readme.as_deref(),
                        &mut reported_missing_catalog,
                    )?;
                    if declaration.kind == Kind::Todo {
                        self.report.todo_kinds += 1;
                    }
                    let uses_todo =
                        declaration.kind == Kind::Todo || declaration.feature == FeatureRef::Todo;
                    if uses_todo {
                        self.report.todo_declarations += 1;
                    }
                    if uses_todo && self.options.deny_todo {
                        self.push(
                            rust_file,
                            header.line,
                            Code::Dt010,
                            format!("test `{name}` uses the reserved word `todo` while --deny-todo is active"),
                        );
                    }
                }
                HeaderTarget::Function {
                    name,
                    is_test: false,
                } => {
                    for declaration in &header.declarations {
                        self.push(
                            rust_file,
                            declaration.line,
                            Code::Dt006,
                            format!("declaration on non-test function `{name}`; only test functions carry DT lines"),
                        );
                    }
                }
                HeaderTarget::OtherItem => {
                    for declaration in &header.declarations {
                        self.push(
                            rust_file,
                            declaration.line,
                            Code::Dt006,
                            "declaration attached to a non-function item".to_string(),
                        );
                    }
                }
                HeaderTarget::Detached => {
                    for declaration in &header.declarations {
                        self.push(
                            rust_file,
                            declaration.line,
                            Code::Dt007,
                            "declaration is separated from its function; only doc lines and attributes may follow it".to_string(),
                        );
                    }
                }
            }
        }
        if has_tests {
            self.report.files_with_tests += 1;
        }
        Ok(())
    }

    /// Extracts the single valid declaration of a test function, reporting structural problems.
    fn declaration_of(
        &mut self,
        rust_file: &Path,
        header: &Header,
        name: &str,
    ) -> Option<Declaration> {
        let Some(first) = header.declarations.first() else {
            self.push(
                rust_file,
                header.line,
                Code::Dt001,
                format!("test `{name}` has no `/// DT[<feature>][<kind>] <statement>` declaration"),
            );
            return None;
        };
        for extra in header.declarations.iter().skip(1) {
            self.push(
                rust_file,
                extra.line,
                Code::Dt006,
                format!("test `{name}` has more than one declaration"),
            );
        }
        if !first.is_first_header_line {
            self.push(
                rust_file,
                first.line,
                Code::Dt007,
                format!("declaration of `{name}` must be the first doc line, before any attribute"),
            );
        }
        match parse_declaration(&first.doc) {
            Ok(declaration) => Some(declaration),
            Err(error @ ParseError::Malformed(_)) => {
                self.push(rust_file, first.line, Code::Dt002, error.to_string());
                None
            }
            Err(error @ ParseError::UnknownKind(_)) => {
                self.push(rust_file, first.line, Code::Dt003, error.to_string());
                None
            }
        }
    }

    /// Resolves the feature reference against the right catalog and reports DT004/DT005/DT008.
    fn check_feature(
        &mut self,
        rust_file: &Path,
        line: usize,
        declaration: &Declaration,
        crate_root: &Path,
        owning_readme: Option<&Path>,
        reported_missing_catalog: &mut bool,
    ) -> Result<(), String> {
        let (readme, id) = match &declaration.feature {
            FeatureRef::Todo => {
                self.report.todo_features += 1;
                return Ok(());
            }
            FeatureRef::Local(id) => {
                let Some(readme) = owning_readme else {
                    if !*reported_missing_catalog {
                        *reported_missing_catalog = true;
                        self.push(
                            rust_file,
                            line,
                            Code::Dt005,
                            format!(
                                "no README.md between this file and the crate root {}; a crate with tests needs a root README with `{SECTION_HEADING}`",
                                crate_root.display()
                            ),
                        );
                    }
                    return Ok(());
                };
                (readme.to_path_buf(), id.clone())
            }
            FeatureRef::Qualified { segments, id } => {
                let target = qualified_readme(crate_root, segments);
                if owning_readme == Some(target.as_path()) {
                    self.push(
                        rust_file,
                        line,
                        Code::Dt008,
                        format!("qualifier `{}` is redundant: it names the module this file already belongs to", declaration.feature),
                    );
                    return Ok(());
                }
                if !target.is_file() {
                    self.push(
                        rust_file,
                        line,
                        Code::Dt008,
                        format!(
                            "qualifier `{}` does not resolve: {} does not exist",
                            declaration.feature,
                            target.display()
                        ),
                    );
                    return Ok(());
                }
                (target, id.clone())
            }
        };

        self.referenced_readmes.insert(readme.clone());
        // Copy the two facts out of the cache so the report can be mutated afterwards.
        let (has_section, has_entry) = {
            let catalog = self.catalog(&readme)?;
            (catalog.has_section, catalog.entries.contains_key(&id))
        };
        if !has_section {
            // A qualified target without a section is always reported; the owning README
            // only once per file.
            let is_owning = owning_readme == Some(readme.as_path());
            if !is_owning || !*reported_missing_catalog {
                if is_owning {
                    *reported_missing_catalog = true;
                }
                self.push(
                    rust_file,
                    line,
                    Code::Dt005,
                    format!("{} has no `{SECTION_HEADING}` section", readme.display()),
                );
            }
            return Ok(());
        }
        if !has_entry {
            self.push(
                rust_file,
                line,
                Code::Dt004,
                format!(
                    "feature point `{id}` is not declared in {} ({SECTION_HEADING})",
                    readme.display()
                ),
            );
        }
        Ok(())
    }
}
