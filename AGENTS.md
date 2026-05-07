# Rust/crates

Ora is an IDE for AI Agent. In the crates folder where the rust code lives:

1. **Code Documentation**: Unless it is a standard, self-explanatory method (e.g., `new()`), every function must include a comment above the signature describing its purpose. Provide inline comments for any complex logic, non-trivial algorithms, or specialized branching within function bodies. Write comments in English.
2. **Explain "Why", not "What"**: Use comments to explain design rationale, business logic constraints, or non-obvious trade-offs. Code structure and naming should inherently describe the "what."
3. **Design for Testability (DfT)**: Favor Dependency Injection and decoupled components. Define interfaces via Traits to allow easy mocking, and prefer small, pure functions that can be unit-tested in isolation.
4. **Prefer Static Dispatch**: Use Generics and Trait Bounds over Trait Objects (e.g., `Box<dyn Trait>`) to leverage monomorphization and compiler optimizations, unless runtime polymorphism is strictly necessary.
5. **Make Illegal States Unrepresentable**: Use Enums with associated data to model state machines, rather than Structs with many optional fields.
6. **No Backward Compatibility**: Prioritize clean design over legacy support. Do **not** preserve compatibility layers "just in case." Break old patterns, remove deprecated code—adapt old to new, never vice versa.

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
- Newly added traits should include doc comments that explain their role and how implementations are expected to use them.
- When writing tests, prefer comparing the equality of entire objects over fields one by one.
- When making a change that adds or changes an API, ensure that the documentation in the `docs/` folder is up to date if applicable.
- Prefer private modules and explicitly exported public crate API.
- Do not create small helper methods that are referenced only once.
- Avoid large modules:
  - Prefer adding new modules instead of growing existing ones.
  - Target Rust modules under 500 LoC, excluding tests.
  - If a file exceeds roughly 800 LoC, add new functionality in a new module instead of extending
    the existing file unless there is a strong documented reason not to.
  - When extracting code from a large module, move the related tests and module/type docs toward
    the new implementation so the invariants stay close to the code that owns them.

## Tests

### Test assertions

- Tests should use pretty_assertions::assert_eq for clearer diffs. Import this at the top of the test module if it isn't already.
- Prefer deep equals comparisons whenever possible. Perform `assert_eq!()` on entire objects, rather than individual fields.
- Avoid mutating process environment in tests; prefer passing environment-derived flags or dependencies from above.

