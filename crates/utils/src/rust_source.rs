//! Syntax-aware Rust source accounting, independent of repository or application policy.
//!
//! Counts nonblank physical lines (including comments) after removing syntax known to be absent
//! from production. Unknown platform/feature cfgs remain included. Macros are not expanded:
//! their source is conservatively counted, and callers should count unreferenced files rather
//! than assuming a macro cannot load them. Module references cover production and test paths.

mod cfg;
#[cfg(test)]
mod tests;

use cfg::{Compilation, enabled, module_paths};
use proc_macro2::Span;
use std::ops::Range;
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// Distinguishes crate roots from ordinary source files when resolving child module paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RustSourceKind {
    CrateRoot,
    Module,
}

/// States whether a module reference can participate in a production build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RustSourceUse {
    Production,
    TestOnly,
}

/// Describes alternative filenames for one declaration, before checking filesystem existence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustModuleReference {
    pub candidates: Vec<PathBuf>,
    pub usage: RustSourceUse,
}

/// Reports source size and module links without reading or changing the filesystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustSourceAnalysis {
    pub production_lines: usize,
    pub modules: Vec<RustModuleReference>,
}

/// Parses a source file and excludes test syntax without truncating later production code.
pub fn analyze_rust_source(
    source: &str,
    filename: &Path,
    kind: RustSourceKind,
) -> syn::Result<RustSourceAnalysis> {
    let syntax = syn::parse_file(source)?;
    // syn strips these prefixes before lexing; restore their byte offset for source masking.
    let prefix_bytes = usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8()
        + syntax.shebang.as_ref().map_or(0, String::len);
    let parent = filename.parent().unwrap_or_else(|| Path::new(""));
    let directory = if kind == RustSourceKind::CrateRoot
        || filename.file_name().is_some_and(|name| name == "mod.rs")
    {
        parent.to_path_buf()
    } else {
        parent.join(filename.file_stem().unwrap_or_default())
    };
    let mut modules = Vec::new();
    let mut excluded = Vec::new();
    for compilation in [Compilation::Production, Compilation::Test] {
        let mut inspector = Inspector {
            compilation,
            parent,
            directories: vec![directory.clone()],
            inline_depth: 0,
            scopes: vec![Scope {
                range: 0..source.len(),
                active: true,
            }],
            excluded: Vec::new(),
            modules: Vec::new(),
            errors: None,
            prefix_bytes,
        };
        inspector.visit_file(&syntax);
        if let Some(error) = inspector.errors {
            return Err(error);
        }
        modules.extend(inspector.modules);
        if compilation == Compilation::Production {
            excluded = inspector.excluded;
        }
    }
    // Byte ranges preserve production syntax sharing a line with a test item, as well as Unicode.
    let mut retained = source.as_bytes().to_vec();
    for mut range in excluded {
        // Separators belong to the removed field/statement but are outside some syn node spans.
        if let Some((offset, byte)) = source.as_bytes()[range.end..]
            .iter()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii_whitespace())
            && matches!(byte, b',' | b';')
        {
            range.end += offset + 1;
        }
        for byte in &mut retained[range] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    let production_lines = retained
        .split(|byte| *byte == b'\n')
        .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()))
        .count();
    let mut unique = Vec::new();
    modules.retain(|module| {
        if unique.contains(module) {
            false
        } else {
            unique.push(module.clone());
            true
        }
    });
    Ok(RustSourceAnalysis {
        production_lines,
        modules,
    })
}

struct Scope {
    range: Range<usize>,
    active: bool,
}

struct Inspector<'a> {
    compilation: Compilation,
    parent: &'a Path,
    directories: Vec<PathBuf>,
    inline_depth: usize,
    scopes: Vec<Scope>,
    excluded: Vec<Range<usize>>,
    modules: Vec<RustModuleReference>,
    errors: Option<syn::Error>,
    prefix_bytes: usize,
}

impl Inspector<'_> {
    /// Restores the enclosing syntax owner after visiting attributes and nested declarations.
    fn scoped(&mut self, span: Span, visit: impl FnOnce(&mut Self)) {
        let active = self.scopes.last().is_some_and(|scope| scope.active);
        let range = span.byte_range();
        self.scopes.push(Scope {
            range: range.start + self.prefix_bytes..range.end + self.prefix_bytes,
            active,
        });
        visit(self);
        self.scopes.pop();
    }

    /// Aggregates malformed cfg diagnostics instead of silently treating broken syntax as tests.
    fn record_error(&mut self, error: syn::Error) {
        match &mut self.errors {
            Some(errors) => errors.combine(error),
            None => self.errors = Some(error),
        }
    }
}

// Attributes must remove their complete syntax owner, not just the attribute or the rest of a file.
macro_rules! scoped_visitors {
    ($($method:ident: $node:ty),* $(,)?) => {
        $(
            /// Associates attributes with their immediate syntax owner while preserving outer cfg.
            fn $method(&mut self, node: &'ast $node) {
                self.scoped(node.span(), |inspector| visit::$method(inspector, node));
            }
        )*
    };
}

impl<'ast> Visit<'ast> for Inspector<'_> {
    scoped_visitors! {
        visit_item: syn::Item,
        visit_impl_item: syn::ImplItem,
        visit_trait_item: syn::TraitItem,
        visit_foreign_item: syn::ForeignItem,
        visit_expr: syn::Expr,
        visit_local: syn::Local,
        visit_field: syn::Field,
        visit_field_value: syn::FieldValue,
        visit_field_pat: syn::FieldPat,
        visit_variant: syn::Variant,
        visit_arm: syn::Arm,
        visit_receiver: syn::Receiver,
        visit_pat_type: syn::PatType,
        visit_pat: syn::Pat,
        visit_bare_fn_arg: syn::BareFnArg,
        visit_bare_variadic: syn::BareVariadic,
        visit_variadic: syn::Variadic,
        visit_stmt_macro: syn::StmtMacro,
        visit_type_param: syn::TypeParam,
        visit_const_param: syn::ConstParam,
        visit_lifetime_param: syn::LifetimeParam,
    }

    /// Masks syntax only when the known cfg makes it impossible in the current compilation.
    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        match enabled(attribute, self.compilation) {
            Ok(false) => {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.active = false;
                    self.excluded.push(scope.range.clone());
                }
            }
            Ok(true) => {}
            Err(error) => self.record_error(error),
        }
    }

    /// Resolves module-path alternatives following Rust's root, inline and explicit-path rules.
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let paths = match module_paths(&module.attrs, self.compilation) {
            Ok(paths) => paths,
            Err(error) => {
                self.record_error(error);
                return;
            }
        };
        // Item visitation normally processes attributes later; classify this declaration first.
        for attribute in &module.attrs {
            self.visit_attribute(attribute);
        }
        let active = self.scopes.last().is_some_and(|scope| scope.active);
        let usage = if self.compilation == Compilation::Production && active {
            RustSourceUse::Production
        } else {
            RustSourceUse::TestOnly
        };
        let anchors = if self.inline_depth == 0 {
            vec![self.parent.to_path_buf()]
        } else {
            self.directories.clone()
        };
        let mut explicit: Vec<PathBuf> = anchors
            .iter()
            .flat_map(|anchor| paths.overrides.iter().map(|relative| anchor.join(relative)))
            .collect();
        let name = module.ident.to_string();
        if module.content.is_some() {
            if paths.default {
                explicit.extend(
                    self.directories
                        .iter()
                        .map(|directory| directory.join(&name)),
                );
            }
            let previous = std::mem::replace(&mut self.directories, explicit);
            self.inline_depth += 1;
            visit::visit_item_mod(self, module);
            self.inline_depth -= 1;
            self.directories = previous;
        } else {
            if paths.default {
                for directory in &self.directories {
                    explicit.push(directory.join(&name).with_extension("rs"));
                    explicit.push(directory.join(&name).join("mod.rs"));
                }
            }
            explicit.sort();
            explicit.dedup();
            self.modules.push(RustModuleReference {
                candidates: explicit,
                usage,
            });
        }
    }
}
