# gitlancer

`gitlancer` is Ora's typed Git CLI runtime. It makes repositories and linked worktrees explicit, separates command construction from execution and parsing, and supports injected runners for deterministic tests.

## Module map

- [domain](src/domain/README.md) defines repository, worktree, path, branch, and commit concepts.
- [exec](src/exec/README.md) owns prepared commands, automation-safe environment, execution, and normalized output.
- [git](src/git/README.md) exposes typed repository, worktree, branch, status, diff, add, commit, push, and configuration use cases.
- [parse](src/parse/README.md) converts stable Git machine output into typed results.

`Git<R: GitRunner>` is the statically dispatched entry point. It owns an execution strategy but no mutable repository state. Production uses the system Git CLI; tests can inject a runner without changing use-case behavior.

The crate deliberately does not use `libgit2`, manage Ora database records, or decide application-level permissions and retries. `GitIntent` exposes read-only, mutating, and network classifications so upper layers can apply policy.

Errors remain separated into domain validation, process execution, output parsing, and bounded diff-size failures before being wrapped as `GitlancerError`. Bounded commands drain both child-process pipes concurrently and terminate Git when a configured output budget is exceeded.

See [Gitlancer Architecture](../../docs/gitlancer-architecture.md) for the detailed design.
