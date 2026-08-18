# Rust/crates

1. **Code Documentation**: Unless it is a standard, self-explanatory method (e.g., `new()`), every function must include a comment above the signature describing its purpose. Provide inline comments for any complex logic, non-trivial algorithms, or specialized branching within function bodies. Write comments in English.
2. **Explain "Why", not "What"**: Use comments to explain design rationale, business logic constraints, or non-obvious trade-offs. Code structure and naming should inherently describe the "what."
3. **Design for Testability (DfT)**: Favor Dependency Injection and decoupled components. Define interfaces via Traits to allow easy mocking, and prefer small, pure functions that can be unit-tested in isolation.
4. **Prefer Static Dispatch**: Use Generics and Trait Bounds over Trait Objects (e.g., `Box<dyn Trait>`) to leverage monomorphization and compiler optimizations, unless runtime polymorphism is strictly necessary.
5. **Make Illegal States Unrepresentable**: Use Enums with associated data to model state machines, rather than Structs with many optional fields.
6. **No Backward Compatibility**: Prioritize clean design over legacy support. Do **not** preserve compatibility layers "just in case." Break old patterns, remove deprecated code—adapt old to new, never vice versa.

Ora is an IDE for AI Agent. In the crates folder where the rust code lives:

- Crate names are prefixed with `ora-`. For example, the `core` folder's crate is named `ora-core`
- When using format! and you can inline variables into {}, always do that.
- Always collapse nested if statements which can be collapsed by &&-combining their conditions.
- Always inline format! args when possible.
- Use method references over closures which only invoke a method on the closure argument and can be replaced by referencing the method directly.
- Avoid bool or ambiguous `Option` parameters that force callers to write hard-to-read code such as `foo(false)` or `bar(None)`. Prefer enums, named methods, newtypes, or other idiomatic Rust API shapes when they keep the callsite self-documenting.
- When you cannot make that API change and still need a small positional-literal callsite in Rust, follow the `argument_comment_lint` convention:
  - Use an exact `/*param_name*/` comment before opaque literal arguments such as `None`, booleans, and numeric literals when passing them by position.
  - Do not add these comments for string or char literals unless the comment adds real clarity; those literals are intentionally exempt from the lint.
  - The parameter name in the comment must exactly match the callee signature.
- When possible, make `match` statements exhaustive and avoid wildcard arms.
- Never hardcode path separators or concatenate path strings manually. Always use `Path`, `PathBuf`, and `.join()` to construct and manipulate filesystem paths.
- Newly added traits should include doc comments that explain their role and how implementations are expected to use them.
- When writing tests, prefer comparing the equality of entire objects over fields one by one.
- When making a change that adds or changes behavior, ensure that the documentation in the `docs/` folder is up to date if applicable.
- Prefer private modules and explicitly exported public crate API.
- Do not create small helper methods that are referenced only once.
- Avoid large modules:
  - Prefer adding new modules instead of growing existing ones.
  - Target Rust modules under 500 LoC, excluding tests.
  - If a file exceeds roughly 800 LoC, add new functionality in a new module instead of extending
    the existing file unless there is a strong documented reason not to.
  - When extracting code from a large module, move the related tests and module/type docs toward
    the new implementation so the invariants stay close to the code that owns them.
- Use local time instead of UTC time.
- Use ora-logging wrapper macros instead of `tracing` macros. Use `ora_logging::clock::now_local` instead of `OffsetDateTime::now_local()`.
- Put logic that is generic — independent of any Ora domain concept, transport, or runtime — in `ora-utils` (`crates/utils`) instead of the calling crate. If you believe a piece of logic is generic, default to placing it in `ora-utils`. `ora-utils` must not depend on any other `ora-*` crate and must not carry domain vocabulary; gate heavier optional dependencies (such as archive formats) behind Cargo features so path-only consumers stay light.
- Before implementing path validation, normalization, or archive extraction, prefer the shared `ora-utils::path` and `ora-utils::archive` capabilities over crate-local logic. If `ora-utils` does not yet provide the required capability, extend `ora-utils` and then consume it instead of implementing it locally in the caller.

## Module READMEs

- Every crate under `crates/` must have an English `README.md` in its crate root.
- Every directory-based production Rust module under a crate's `src/` tree must have an English `README.md` in the module directory. Single-file modules do not need to be converted into directories; their responsibilities should be covered by the nearest parent README.
- Test, fixture, generated, example, and other non-production directories do not require module READMEs.
- `crates/contracts`, `crates/domain`, and `crates/pty`, including their descendant modules, are intentional exceptions. Their type definitions and code-level documentation are the primary documentation because they do not own architectural orchestration that benefits from separate README files.
- When adding a crate or directory-based module, add its README in the same change. When changing a module's responsibilities, boundaries, core flows, or interactions, update the corresponding README in the same change.
- READMEs document stable facts that should remain true across internal refactors: responsibilities, non-responsibilities, public boundaries, key invariants, lifecycle, important failure semantics, and module interactions. Put local implementation rationale, algorithm details, data-structure choices, specialized branches, performance trade-offs, and temporary implementation constraints in English code comments instead.
- A README that owns tests must also contain a `## Feature points` section; see "Developer test (DT) declarations".

## Developer test (DT) declarations

Every Rust test function under `crates/` carries a one-line structured declaration as the first line of its doc comment. `task lint:dt` (optionally scoped: `task lint:dt -- crates/fs`) rejects any test that lacks or misspells it. The declaration itself cannot be omitted; the only escape hatch is the reserved word `todo` in either field.

```rust
/// DT[<feature>][<kind>] <statement>
#[test]
fn …() { … }
```

- `<feature>` is a kebab-case feature point declared in the owning module README under `## Feature points`. The owning README is the nearest `README.md` walking up from the test file; crate-level `tests/` files resolve to the crate root README. Prefix with a `src/`-relative module path (`parse::commit-readback`) only when the test must attach to another module's catalog in the same crate; never qualify with the module the file already lives in.
- `<kind>` is exactly one of `happy` (nominal path), `edge` (boundary of valid input: empty, max, repeat, ordering, exhaustive combinations), `error` (rejection, failure handling, compensation, no dirty state), `concurrency` (interleaving, cancellation, timeouts, locks, leases). Do not encode provenance such as "regression" here; put it in the statement if it matters.
- `todo` may replace `<feature>`, `<kind>`, or both when no existing entry fits and reworking the catalog does not belong in the current change. It passes the check, is counted as governance debt, and must never be a default. Never add `todo` to a README catalog.
- `<statement>` is one English sentence in present tense: under what condition, what outcome. Describe the behavior, not the test ("A stale lease is reclaimed…", not "Verifies that…"). Do not repeat the feature point, the kind, or the function name.

Rules:

- One declaration per test function, on the first `///` line, before any `#[…]` attribute. Additional `///` lines may follow and are ignored by tooling.
- Helpers, fixtures, and non-test functions must not carry a DT line.
- Every test attaches to exactly one feature point and one kind. If a test spans several feature points, pick the primary one; if that happens often, split the test.
- Before using a new feature point, add it to the owning README's `## Feature points` section as ``- `id`: description``. Read the existing entries first and reuse one when the meaning overlaps. Name behavior areas with noun phrases (`branch-listing`), not test actions, and do not prefix with the module name.
- Every crate that has tests must have a crate root `README.md` with a `## Feature points` section, including `crates/contracts`, `crates/domain`, and `crates/pty`; their exemption from module-level READMEs is unchanged.
- Do not generate test functions with `macro_rules!`; the checker treats a DT line inside a macro body as one declaration shared by every expansion.
- Renaming or removing a feature point is a single change that updates the README and every declaration referring to it.

## Tests

`task test` runs the frontend and Rust workspace lint and test tasks. It can take a
long time, so prefer the smallest relevant task while iterating and run the full task
before considering a repository-wide change complete. Use `task --list` to see the
authoritative list of available tasks.

- Format changed files: `task format`
- Frontend lint: `task lint:frontend`
- Frontend tests: `task test:frontend`
- Rust workspace lint: `task lint:crates`
- Rust workspace tests: `task test:crates`
- DT declaration check (standalone, scoped by path): `task lint:dt -- crates/fs`
- All lint tasks: `task lint`
- All lint and test tasks (long-running): `task test`

### Test assertions

- Tests should use pretty_assertions::assert_eq for clearer diffs. Import this at the top of the test module if it isn't already.
- Prefer deep equals comparisons whenever possible. Perform `assert_eq!()` on entire objects, rather than individual fields.
- Avoid mutating process environment in tests; prefer passing environment-derived flags or dependencies from above.
- When testing structured events, logs, or spans using `tracing`, always install a test-scoped subscriber/dispatcher with an explicit `LevelFilter::TRACE` (or the required minimum level). Use `tracing::subscriber::with_default` or `tracing::dispatcher::with_default` to isolate the subscriber to the current test thread. Keep every operation that can emit the same `tracing` callsites under that scoped subscriber, including setup helpers, bootstrap code, repository fixtures, and API-surface smoke checks that create spans or events. This matters even for tests that do not assert logs directly: `tracing` caches callsite interest, so a normal test that touches a callsite first can make a later structured-log assertion fail intermittently. Prefer shared helpers such as `with_trace_logging` / `with_recorded_trace_logging` so ordinary tests and recording tests use the same scoped TRACE setup.
