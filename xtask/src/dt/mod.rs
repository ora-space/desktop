//! `cargo xtask check-dt`: validates developer test (DT) declarations and the
//! `## Feature points` catalogs they resolve against. See `README.md` in this
//! directory for the rules and the design document they come from.

mod catalog;
mod check;
mod declaration;
mod resolve;
mod scan;

#[cfg(test)]
mod tests;

use check::{CheckOptions, CheckReport, check};
use resolve::collect_scope;
use std::path::{Path, PathBuf};

/// Default scope when no path is given.
const DEFAULT_SCOPE: &str = "crates";

/// Parsed command line of `check-dt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckDtArgs {
    /// Scope paths relative to the workspace root (or absolute); empty means `crates/`.
    pub paths: Vec<PathBuf>,
    /// Treat `todo` placeholders as violations.
    pub deny_todo: bool,
}

impl CheckDtArgs {
    /// Parses `[--deny-todo] [PATH...]`.
    pub fn parse<I: IntoIterator<Item = String>>(arguments: I) -> Result<Self, String> {
        let mut args = Self {
            paths: Vec::new(),
            deny_todo: false,
        };
        for argument in arguments {
            match argument.as_str() {
                "--deny-todo" => args.deny_todo = true,
                flag if flag.starts_with('-') => {
                    return Err(format!(
                        "unknown option `{flag}`; usage: cargo xtask check-dt [--deny-todo] [PATH...]"
                    ));
                }
                path => args.paths.push(PathBuf::from(path)),
            }
        }
        Ok(args)
    }
}

/// Runs the check, prints diagnostics to stderr, and fails when any rule fired.
pub fn run_check_dt(workspace_root: &Path, args: &CheckDtArgs) -> Result<(), String> {
    let scope: Vec<PathBuf> = if args.paths.is_empty() {
        vec![workspace_root.join(DEFAULT_SCOPE)]
    } else {
        args.paths
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    workspace_root.join(path)
                }
            })
            .collect()
    };
    let files = collect_scope(&scope)?;
    let report = check(
        &files,
        CheckOptions {
            deny_todo: args.deny_todo,
        },
    )?;
    eprint!("{}", render_report(workspace_root, &report));
    if report.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} DT violation(s) across {} file(s)",
            report.diagnostics.len(),
            distinct_files(&report)
        ))
    }
}

/// Formats diagnostics as `path:line:1: CODE message`, followed by the summary lines.
fn render_report(workspace_root: &Path, report: &CheckReport) -> String {
    let mut output = String::new();
    for diagnostic in &report.diagnostics {
        let path = diagnostic
            .path
            .strip_prefix(workspace_root)
            .unwrap_or(&diagnostic.path);
        output.push_str(&format!(
            "{}:{}:1: {} {}\n",
            path.display(),
            diagnostic.line,
            diagnostic.code,
            diagnostic.message
        ));
    }
    output.push_str(&format!(
        "note: {} declaration(s) use `todo` (feature: {}, kind: {})\n",
        report.todo_declarations, report.todo_features, report.todo_kinds
    ));
    if report.diagnostics.is_empty() {
        output.push_str(&format!(
            "ok: checked {} test function(s) in {} file(s)\n",
            report.tests_checked, report.files_with_tests
        ));
    }
    output
}

/// Number of distinct files that carry at least one diagnostic.
fn distinct_files(report: &CheckReport) -> usize {
    let mut paths: Vec<&Path> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.path.as_path())
        .collect();
    paths.sort();
    paths.dedup();
    paths.len()
}
