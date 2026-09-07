# Architecture checks

These checks protect the integration refactor's ownership rules. They do not approve new product
semantics or replace behavior tests. The root-decision review remains skipped for this refactor
at Eric's request.

| Task                   | Enforced fact                                                                                                | Normal repair                                                                                                     |
| ---------------------- | ------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| `task check:contracts` | Logical catalog uniqueness, Desktop bindings/authorization, owned generated artifacts and error-schema drift | Correct the owning declaration, then `task export-contracts`                                                      |
| `task check:features`  | Named public feature exports, real symbol existence, shared-state direction and single resource composition  | Use the owning interface or move genuinely shared responsibility; see [frontend ownership](frontend-ownership.md) |
| `task check:rust-size` | Production source size and ratcheting legacy exceptions                                                      | Extract a coherent responsibility with its tests, then lower/remove its exception                                 |

Feature checks run in `lint:frontend`; Rust size checks run in `lint:crates`. Both therefore run in
`task lint` and `task test`. Contract checks run in `test:frontend`; Tauri compilation/build checks
still verify real handler signatures and capability coverage. A successful catalog check alone
is not proof that a handler compiles or that its runtime behavior is correct.

## Rust source accounting

`ora-utils` exposes a feature-gated, domain-free `rust_source` parser. `xtask` owns the Cargo/Git
inventory, thresholds and baseline. The metric is **nonblank physical source lines, including
comments, after removing syntax known to be absent from production**. It is a navigation/growth
indicator, not a measure of module depth or a reason to delete documentation.

The parser uses syntax spans rather than truncating at the first `#[cfg(test)]`. It handles
inline tests, attributed members/fields/statements, file-level attributes, `all`/`any`/`not` and
`cfg_attr`, including code that follows a test module and production/test syntax sharing a line.
Unicode, BOM and shebang offsets are preserved. Unknown feature/platform predicates are retained
whenever production is possible, so running on Linux does not hide Windows implementation.
Unexpanded macros are conservatively counted; strings and comments resembling test syntax are
not interpreted as attributes.

Cargo target roots and actual module references identify out-of-line test files and their
helpers. Integration-test/benchmark targets are test-only; a file also reached by production is
counted. Module filenames and explicit paths follow the [Rust module source rules](https://doc.rust-lang.org/reference/items/modules.html#module-source-filenames),
including custom root filenames, legacy `mod.rs`, nested inline directories and conditional test
paths. This checker does not introduce new `mod.rs` files.

All tracked and nonignored untracked `.rs` files within Cargo workspace package directories are
inventoried. Canonical containment checks prevent a source link from expanding the inventory
outside the workspace. Unreferenced source is conservatively production; its explicit test-only
children still follow their declarations. Unreachable cycles are counted, not treated as an
exemption. Compiler-generated macro expansion is not measured; handwritten macro bodies are.
The check reads source and never changes the baseline or generated files.

At the initial checkpoint: 671 source files, 80 test-only files, 30 files above the 500-line target,
and 7 above the hard 800-line threshold. For comparison, `backend/src/task.rs` is 383 production
lines despite an early out-of-line test declaration; its lifecycle test file is excluded.
`bootstrap.rs` is 316 by this metric (earlier progress entries used approximate physical totals).

## Existing size debt

The target remains below 500 production lines. **800 is a hard gate for new over-limit modules**,
not a recommendation to grow everything to 800. The following pre-existing modules are the only
initial exceptions; the owner names mean the maintainers of that Cargo crate, not newly assigned
individuals. Concrete extraction directions live beside the exact counts in
[`xtask/rust-size-baseline.json`](../xtask/rust-size-baseline.json).

| Existing module                                | Owner             | Production lines | Next extraction                                                         |
| ---------------------------------------------- | ----------------- | ---------------: | ----------------------------------------------------------------------- |
| `backend/src/agent_runtime.rs`                 | `ora-backend`     |             1180 | Durable binding/history adoption and command admission                  |
| `backend/src/plugin.rs`                        | `ora-backend`     |             1053 | Private host configuration and gateway/projection responsibilities      |
| `db/src/repository/workflow_run_engine.rs`     | `ora-db`          |             1051 | Transaction-preserving run, node/history and artifact repository groups |
| `backend/src/agent_runtime/actor.rs`           | `ora-backend`     |             1007 | Active-turn settlement and idle controls, sharing one actor state       |
| `application/src/workflow_run/engine/graph.rs` | `ora-application` |              879 | Parsing/validation and indexed traversal                                |
| `backend/src/workflow/run/executor.rs`         | `ora-backend`     |              838 | Turn-output collection and baseline/file-change projection              |
| `backend/src/agent_runtime/connection.rs`      | `ora-backend`     |              804 | Handshake/authentication separated from supervision                     |

Growth above an exception fails. Shrinking an oversized file also requires lowering its recorded
count, so removed debt cannot grow back unnoticed. Once it reaches 800 or is deleted, remove the
exception. Stale entries, mismatched owners and missing split plans fail. Do not regenerate this
file to approve growth; changes to exceptions need explicit review and rationale.

`cargo xtask report-rust-size` prints the current inventory as JSON for inspection without
approving exceptions. Parser tests cover source syntax; graph tests cover outlined tests and
shared production references; policy tests cover new growth, ratcheting and stale/unowned debt.
Rust's private modules and explicit crate exports remain compiler-enforced. Public interface
changes must still be reviewed against the domain-handle ownership documented in the Backend
README; passing a size check does not justify exposing repositories, locks or supervisors.
