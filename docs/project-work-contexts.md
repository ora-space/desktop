# Project Work Contexts

A project work context records which client window is actively working in which project. It is a renewable lease, not permanent ownership, so a crashed or disconnected client cannot lock a project forever.

## Persisted shape

One `project_work_contexts` row exists per active client window, holding `id`, `surface`, `window_id`, `project_id`, `lease_expires_at`, `created_at`, and `updated_at`. `(surface, window_id)` is unique, so a window has at most one context row and reopening refreshes that stable row instead of competing with itself.

`surface` is `web` or `tauri`. The `projects` table stays limited to project identity and never becomes the source of truth for active working context: a loaded `Project` carries identity and audit fields only.

`ProjectWorkContext` has no soft-delete column. Expired rows are deleted outright rather than flagged.

## Lease timing

The backend owns lease timing. `OpenProjectWorkContextHandler` and `RenewProjectWorkContextHandler` compute `lease_expires_at` as backend time plus **120 seconds**; clients never supply an absolute expiry. Every open, switch, and renew writes a fresh non-expired lease as part of the same operation, so a context is never briefly established with a stale expiry while waiting for the next heartbeat.

Clients own heartbeat scheduling and their own window identity; the handlers own only lease duration and conflict policy. No client renews today — see [Current wiring](#current-wiring) — so a heartbeat interval comfortably inside the 120-second window still has to be chosen when a caller is added.

## Occupancy rules

At most one non-expired context may exist per `(surface, window_id)` pair.

- The same window switching from one project to another updates its existing row rather than creating a second active row.
- Exclusivity applies **between Tauri windows only**: a request is rejected as a conflict when the requesting surface is `Tauri` and a different non-expired `Tauri` window already holds the project. A `Web` request never conflicts, and a `Web` context never blocks a Tauri window.
- Expired rows never block a claim. Conflict detection considers only contexts whose `lease_expires_at` is still in the future relative to backend time.

Conflict responses tell the client only that the project is occupied. The owning `surface` and `window_id` appear in backend logs, not in the client-facing payload. Over HTTP an occupied project is a `409`.

`RenewProjectWorkContextHandler` fails when no context exists for the requested surface and window; renewal never implicitly creates one.

## Retention of expired rows

Expired rows are intentionally retained so operators can inspect recent context history, and they stay invisible to active-ownership checks the whole time. `ProjectWorkContextRepository::delete_expired_project_work_contexts` takes an explicit cutoff and removes rows older than it, leaving the retention window to the caller rather than hard-coding it in the port.

Aggregate project deletion also cascades through that project's context rows.

## Transport support

Work contexts are deliberately outside `ora-backend`. The Web server keeps its own `ProjectWorkContextApi` composed directly from the shared repository pool, and exposes:

- `POST /api/project-work-contexts/open`
- `POST /api/project-work-contexts/renew`

Desktop does not implement these operations. Its contracts transport rejects `openProjectWorkContext` and `renewProjectWorkContext` with `unsupported_operation` before any Tauri command is invoked.

## Current wiring

The persistence, lease, and occupancy rules above are fully implemented, but the surrounding lifecycle is not yet connected:

- The only production caller is Web bootstrap, which opens one synthetic context for `surface = web`, `window_id = main` against the configured bootstrap project before readiness is reported.
- No frontend calls `projectWorkContext.open` or `projectWorkContext.renew`, so no lease is ever renewed and the bootstrap context expires two minutes after startup. Occupancy enforcement therefore does not currently reject anything in practice.
- `delete_expired_project_work_contexts` has no scheduled caller, so expired rows accumulate. Wiring it as an `ora-scheduler` job at a composition root, which would own the retention cutoff, is outstanding work.

See [Application and Contracts Boundary](application-contracts.md), [Web Server Runtime](web-server-runtime.md), and [Desktop Runtime](desktop-runtime.md).
