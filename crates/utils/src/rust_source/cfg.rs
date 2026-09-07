//! Conservative conditional-compilation analysis across both test and production builds.

use std::path::PathBuf;
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, Lit, Meta, Token};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Compilation {
    Production,
    Test,
}

/// Tracks both possibilities because platform and feature flags are intentionally unspecified.
#[derive(Clone, Copy)]
struct Possibility {
    yes: bool,
    no: bool,
}

/// Evaluates only known test predicates, retaining every potentially supported production config.
fn predicate(meta: &Meta, compilation: Compilation) -> syn::Result<Possibility> {
    match meta {
        Meta::Path(path) if path.is_ident("test") => Ok(Possibility {
            yes: compilation == Compilation::Test,
            no: compilation == Compilation::Production,
        }),
        Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
            let children = list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?
                .iter()
                .map(|child| predicate(child, compilation))
                .collect::<syn::Result<Vec<_>>>()?;
            if list.path.is_ident("all") {
                Ok(Possibility {
                    yes: children.iter().all(|child| child.yes),
                    no: children.iter().any(|child| child.no),
                })
            } else {
                Ok(Possibility {
                    yes: children.iter().any(|child| child.yes),
                    no: children.iter().all(|child| child.no),
                })
            }
        }
        Meta::List(list) if list.path.is_ident("not") => {
            let child = predicate(&list.parse_args::<Meta>()?, compilation)?;
            Ok(Possibility {
                yes: child.no,
                no: child.yes,
            })
        }
        _ => Ok(Possibility {
            yes: true,
            no: true,
        }),
    }
}

/// Expands only relevant built-in cfg attributes; unknown procedural attributes remain opaque.
fn meta_enabled(meta: &Meta, compilation: Compilation) -> syn::Result<bool> {
    match meta {
        Meta::Path(path) if path.is_ident("test") || path.is_ident("bench") => {
            Ok(compilation == Compilation::Test)
        }
        Meta::List(list) if list.path.is_ident("cfg") => {
            Ok(predicate(&list.parse_args::<Meta>()?, compilation)?.yes)
        }
        Meta::List(list) if list.path.is_ident("cfg_attr") => {
            let arguments =
                list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            let mut arguments = arguments.iter();
            let condition = arguments
                .next()
                .ok_or_else(|| syn::Error::new_spanned(meta, "cfg_attr requires a condition"))?;
            let condition = predicate(condition, compilation)?;
            if condition.no {
                return Ok(true);
            }
            for attribute in arguments {
                if !meta_enabled(attribute, compilation)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(true),
    }
}

/// Determines whether an attributed syntax node may exist in the selected compilation.
pub(super) fn enabled(attribute: &Attribute, compilation: Compilation) -> syn::Result<bool> {
    meta_enabled(&attribute.meta, compilation)
}

/// Records possible explicit paths and whether the ordinary module filename can still apply.
#[derive(Default)]
pub(super) struct Paths {
    pub overrides: Vec<PathBuf>,
    pub default: bool,
}

/// Preserves both default and alternate paths when a platform/feature predicate is unknown.
fn collect_paths(
    meta: &Meta,
    compilation: Compilation,
    certain: bool,
    paths: &mut Paths,
) -> syn::Result<()> {
    match meta {
        Meta::NameValue(value) if value.path.is_ident("path") => {
            let Expr::Lit(expression) = &value.value else {
                return Err(syn::Error::new_spanned(
                    meta,
                    "module path must be a string literal",
                ));
            };
            let Lit::Str(value) = &expression.lit else {
                return Err(syn::Error::new_spanned(
                    meta,
                    "module path must be a string literal",
                ));
            };
            paths.overrides.push(PathBuf::from(value.value()));
            paths.default &= !certain;
        }
        Meta::List(list) if list.path.is_ident("cfg_attr") => {
            let arguments =
                list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            let mut arguments = arguments.iter();
            let condition = arguments
                .next()
                .ok_or_else(|| syn::Error::new_spanned(meta, "cfg_attr requires a condition"))?;
            let condition = predicate(condition, compilation)?;
            if condition.yes {
                for attribute in arguments {
                    collect_paths(attribute, compilation, certain && !condition.no, paths)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Reads module source-path alternatives without depending on the host platform's active cfg.
pub(super) fn module_paths(
    attributes: &[Attribute],
    compilation: Compilation,
) -> syn::Result<Paths> {
    let mut paths = Paths {
        default: true,
        ..Paths::default()
    };
    for attribute in attributes {
        collect_paths(
            &attribute.meta,
            compilation,
            /*certain*/ true,
            &mut paths,
        )?;
    }
    Ok(paths)
}
