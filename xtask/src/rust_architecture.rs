//! Repository policy for production Rust module size; parsing remains in domain-free ora-utils.

mod sources;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const HARD_LIMIT: usize = 800;
const TARGET: usize = 500;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ModuleSize {
    owner: String,
    production_lines: usize,
    test_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    modules: BTreeMap<PathBuf, Exception>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exception {
    owner: String,
    lines: usize,
    split_plan: String,
}

/// Reports source accounting without rewriting or approving any oversized-module exception.
pub fn report_rust_architecture(workspace: &Path) -> Result<()> {
    let modules = sources::collect(workspace)?;
    println!("{}", serde_json::to_string_pretty(&modules)?);
    Ok(())
}

/// Enforces new-module limits and the reviewed, ratcheting baseline for existing large modules.
pub fn check_rust_architecture(workspace: &Path) -> Result<()> {
    let modules = sources::collect(workspace)?;
    let baseline: Baseline = serde_json::from_str(&fs::read_to_string(
        workspace.join("xtask").join("rust-size-baseline.json"),
    )?)?;
    let errors = violations(&modules, &baseline);
    if !errors.is_empty() {
        return Err(errors.join("\n").into());
    }
    let test_only = modules.values().filter(|module| module.test_only).count();
    let above_target = modules
        .values()
        .filter(|module| module.production_lines > TARGET)
        .count();
    println!(
        "Rust module size: {} files, {test_only} test-only, {above_target} above the {TARGET}-line target; {} reviewed exceptions above {HARD_LIMIT}.",
        modules.len(),
        baseline.modules.len()
    );
    Ok(())
}

/// Rejects growth, stale exceptions and unowned debt rather than silently regenerating a baseline.
fn violations(modules: &BTreeMap<PathBuf, ModuleSize>, baseline: &Baseline) -> Vec<String> {
    let mut errors = Vec::new();
    for (path, module) in modules {
        if module.production_lines > HARD_LIMIT && !baseline.modules.contains_key(path) {
            errors.push(format!(
                "{}: new oversized production module ({} > {HARD_LIMIT}); split by responsibility",
                path.display(),
                module.production_lines
            ));
        }
    }
    for (path, exception) in &baseline.modules {
        let Some(module) = modules.get(path) else {
            errors.push(format!(
                "{}: remove the exception for a deleted module",
                path.display()
            ));
            continue;
        };
        if module.production_lines <= HARD_LIMIT {
            errors.push(format!(
                "{}: remove the exception now that the module is within {HARD_LIMIT} lines",
                path.display()
            ));
        } else if module.production_lines > exception.lines {
            errors.push(format!(
                "{}: production module grew from {} to {} lines; extract the new responsibility",
                path.display(),
                exception.lines,
                module.production_lines
            ));
        } else if module.production_lines < exception.lines {
            errors.push(format!(
                "{}: lower the baseline from {} to {} so removed debt cannot grow back",
                path.display(),
                exception.lines,
                module.production_lines
            ));
        }
        if exception.owner != module.owner || exception.split_plan.trim().len() < 30 {
            errors.push(format!(
                "{}: exception needs the owning crate ({}) and a concrete split plan",
                path.display(),
                module.owner
            ));
        }
    }
    errors
}
