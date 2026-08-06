# Git Output Parsing Module

This module converts stable Git porcelain and plumbing output into gitlancer domain values and typed use-case responses.

## Supported boundaries

- Commit parsing reads the object-id and the following summary line into `CommitId` and `CommitResponse`; the summary may be empty.
- Status parsing consumes porcelain-v2 NUL-delimited records, ignores headers, and preserves each machine record as a `StatusEntry`.
- Worktree parsing consumes `git worktree list --porcelain`, associates optional branch refs, and distinguishes the main checkout from linked worktrees.

An empty or structurally incomplete required payload returns `ParseError`; parsers do not invent missing identities. Detached worktrees are represented by an absent branch rather than rejected.

The module assumes command builders requested the documented stable formats. It does not execute Git, interpret localized human messages, apply application policy, or validate filesystem existence.

See the [gitlancer overview](../../README.md) and [Gitlancer Architecture](../../../../docs/gitlancer-architecture.md).
