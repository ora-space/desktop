# Frontend Architecture

English | [中文](frontend-architecture.zh.md)

The App Shell (`packages/app-shell`) is organized by feature. This document covers the ownership
rules that cut across features: translations, query caches, public feature interfaces and the test
transport. Feature internals are described in each feature directory's `README.md`.

## Translation resources

Feature copy lives in data-only `features/<feature>/translations.ts` entries; domain-specific
contract errors live beside the feature's copy. Only genuinely shared shell and transport messages
remain in `i18n/common-resources.ts`. Settings splits general settings, plugins, Skills and Roles
behind its own entry; workflow editor copy belongs to the editor even where historical keys start
with `settings.workflow`; shared Cut, Copy, Paste and Select All copy belongs to
`features/editor/translations.ts`.

`i18n/resources.ts` composes those entries explicitly. It imports no React or i18next code, and
resources never import their feature implementation. `composeTranslationResources` rejects
duplicate logical-key ownership, missing language keys and incomplete plural groups (English
`_one`/`_other` variants and Chinese unsuffixed count messages are compared by logical key).

`i18n/i18n-instance.ts` is the sole `appI18n` initializer. It composes all copy synchronously
(`initAsync: false`), keeps the Chinese fallback and the `ora.locale` storage and document-language
behavior. Features never register resources when they mount. Any test that renders
`useTranslation` imports `appI18n` itself.

## Query and invalidation ownership

`state/data/` owns cache identity and shared data rules. There is no global key registry or barrel:
consumers import the owner they need.

| Owner                                                                                | Responsibility                                                                                                                  |
| ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `workspace.ts`, `sessions.ts`                                                        | Shared lists, adopting authoritative responses, scrubbing after aggregate deletion, session tree placement                      |
| `workflows.ts`, `workflow-analysis.ts`, `workflow-runs.ts`, `workflow-run-retry.ts`  | Definition, draft, version and execution-analysis queries; persisted run queries, mutations, projections and retry presentation |
| `mock-workflows.ts`, `mock-workflow-runs.ts`, `mock-workflow-mounts.ts`              | Memory-runtime identities and subscriptions, kept apart from persisted run caches                                               |
| `files.ts`, `file-watch.ts`, `diff.ts`, `workspace-status.ts`, `workspace-review.ts` | Scoped file access, watcher-driven invalidation, workspace diff, status and review refresh                                      |
| `plugins.ts`, `agent-runtime.ts`, `plugin-lifecycle.ts`, `mcp-health.ts`             | Catalog and configuration caches, per-agent model prefixes, cross-domain lifecycle coordination, MCP health                     |
| `agents.ts`, `skills.ts`, `settings.ts`, `identity.ts`                               | Definition, preference and identity caches                                                                                      |

UI hooks own selection, draft and composer cleanup, mutation activity and view-specific actions;
they apply authoritative results through the data owner before coordinating UI state. Rules worth
knowing:

- Project deletion scrubs main-workspace and task sessions even when the task workspace list has not
  loaded.
- List renames patch the response and invalidate with `refetchType: "none"`; standalone deletes
  refresh active lists, while parent cascades may defer child refreshes. Do not replace this with a
  blanket invalidation.
- The application event hook owns reconnect and abort state but delegates cache rules. A `Ready`
  event or a disconnect refetches sessions to close missed-event gaps; a title event only
  invalidates them.
- Persisted run detail keys are `['workflowRun', 'detail', runId]`; the memory runtime keeps
  `['workflowRun', runId]`. Invalidations never merge the two spaces. Terminal-status
  classification lives in `@ora/workflow-runtime`.
- Files owns the explicit Explorer/Search composition and the task-worktree versus project-checkout
  choice. Rename events invalidate both paths, their parents and searches; changing scope aborts the
  old watcher and unmount aborts the current one.

Plugin refreshes are scoped deliberately:

| Trigger | Invalidated |
|---|---|
| Agent activation | Runtime availability and that agent's discovery queries in every workspace |
| Agent stop or removal | Runtime availability, not discovery against the stopped runtime |
| Settled plugin mutation | Installed and available plugins, and this session's Hook lifecycle results |
| `PluginStatusChanged` | Installed plugins, runtime availability and Skills; not available plugins or models |
| `AgentModelsInvalidated` | Only that agent's discovery-query prefix, cancelling requests in flight first |
| Explicit Hook initialization | Only the Hook lifecycle results |

Data tests use real `QueryClient` caches and observers to verify exact invalidation and refetch
behavior.

## Public feature interfaces

Each feature's `interface.json` records its owner and the named exports other features or shell
composition may consume, each with the reason it is shared. Everything else is private, including
other exports from the same file. This is a visibility policy, not a runtime registry: callers keep
direct named imports and no all-feature barrel exists. Do not list every export or add wildcards to
make the check pass.

`task check:features` (part of `lint:frontend`) resolves imports with the App Shell TypeScript
configuration and checks the symbols actually imported across `apps/` and `packages/`: static and
dynamic imports, re-exports, aliases, inline type imports, CommonJS and Vitest mocks. Namespace or
whole-module access to a foreign feature is rejected because it would expose private exports, and
so is nonliteral module access. A test may mock explicitly named public UI exports with a
zero-argument object-literal factory; spreading the original module is not an escape hatch.

Shared `state/` code cannot import feature UI even when that UI is public. For example, the agent
catalog lives in `state/hooks/use-agent-catalog.ts`, review sizing in
`state/stores/review-layout.ts`, composer quote actions in
`state/actions/add-composer-file-selection.ts`, and workflow viewport constraints in
`workflow-node-chrome/viewport.ts`. Feature translations are consumed from outside their owner only
by `i18n/resources.ts`.

## Typed test transport

`test/contracts-transport.ts` provides `createTestClient(handlers)`, which uses the production
generated client. `TestHandlers` derives each request, response and unary/stream mode from the
operation catalog, so fixtures never restate the namespace map. The transport rejects omitted or
unknown operations and wrong response modes; stream lookup happens on first consumption and
iterator exit forwards cleanup to the handler. DTOs, bigints and `AbortSignal` options cross the
seam unchanged.

`test/memory/` owns domain-specific state, constructors and operation handlers. A test composes only
the domains it needs:

```typescript
const state = createWorkspaceMemory();
const client = createTestClient(workspaceHandlers(state));
```

Synthetic fixtures such as `emptyFilesHandlers` and `readyEffectHandlers` describe their behavior in
their names and are never global defaults; a test with no dependencies starts from
`createTestClient({})`. Custom behavior, including scripted chat streams and injected failures,
uses typed handlers. Never replace generated client methods or cast partial objects to
`ContractsClient`. Handler assertions include the transport's second `options` argument.
