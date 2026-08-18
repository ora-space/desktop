# DT declaration checker

`cargo xtask check-dt [--deny-todo] [PATH...]` validates developer test (DT) declarations under
`crates/` (or the given paths) against the feature point catalogs in module READMEs. It is the
enforcement half of the Rust test asset governance design; the writing rules live in the repository
`AGENTS.md` under "Developer test (DT) declarations".

## Responsibilities

- Find every test function (`#[test]`, `#[tokio::test]`, or any attribute whose path ends in `test`)
  in the scoped `.rs` files and require exactly one well-formed `/// DT[<feature>][<kind>] <statement>`
  line as the first line of its header block.
- Resolve each feature reference to a README: the nearest `README.md` from the file's directory up to
  the crate root, or `<crate>/src/<segments>/README.md` for a `seg::seg::id` qualifier.
- Parse the `## Feature points` section of every resolved or in-scope README and reject unknown ids,
  malformed entries, duplicates, and the reserved word `todo` as an entry.
- Print `path:line:1: DTnnn message` diagnostics sorted by path and line, a `note:` line counting
  `todo` placeholders, and exit non-zero when any rule fired.

## Non-responsibilities

- No Rust parsing: the scanner is line based (see "Blind spots").
- No aggregation, statistics, or gap reports; those consume the same parser but are separate tooling.
- No incremental mode. Scope is an explicit path list; the default is the whole `crates/` tree.

## Rules

| Code  | Trigger                                                                                                    |
| ----- | ---------------------------------------------------------------------------------------------------------- |
| DT001 | Test function without a declaration                                                                        |
| DT002 | `DT[` line that does not match the grammar                                                                 |
| DT003 | Kind outside `happy`, `edge`, `error`, `concurrency`, `todo`                                               |
| DT004 | Feature id not declared in the resolved catalog                                                            |
| DT005 | Owning README missing, or resolved README has no `## Feature points` section (once per file for the owner) |
| DT006 | More than one declaration on a test, or a declaration on a non-test function or non-function item          |
| DT007 | Declaration not on the first header line, or separated from its function by a blank line                   |
| DT008 | Qualifier that names the file's own module (redundant) or a module without a README                        |
| DT009 | Malformed catalog entry, duplicate id, empty description, or `todo` used as an id                          |
| DT010 | Any `todo` placeholder while `--deny-todo` is set                                                          |

`todo` in either field is otherwise valid; it is counted and reported, never rejected.

## Module layout

- `declaration.rs`: grammar of one declaration line.
- `scan.rs`: header-block scanner that pairs declarations with the item they precede.
- `catalog.rs`: `## Feature points` parser.
- `resolve.rs`: scope walking, crate root and owning README resolution.
- `check.rs`: rule evaluation over scanned files and catalogs.
- `mod.rs`: argument parsing and report rendering.

## Blind spots

- `#[test]` or `/// DT[` inside string literals or block comments is treated as real.
- A DT line inside a `macro_rules!` body is one declaration shared by every expansion.

Both are accepted because the repository does not use these patterns and the failure is loud.

## Feature points

Stable identifiers that DT declarations in this module attach to.

- `declaration-grammar`: Parsing one `DT[<feature>][<kind>] <statement>` line, including qualifiers and `todo`.
- `header-scanning`: Pairing doc lines and attributes with the function or item they precede.
- `catalog-parsing`: Reading `## Feature points` entries from a README and reporting format problems.
- `ownership-resolution`: Choosing the crate root and owning README for a file, and collecting scope files.
- `rule-evaluation`: Turning scanned headers and catalogs into DT001–DT010 diagnostics.
- `command-line`: Argument parsing for `check-dt`.
