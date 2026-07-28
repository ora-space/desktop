# Project Work Context Application Module

This module coordinates the project currently opened by a client surface and window. A work context is a renewable lease, not permanent project ownership.

## Lifecycle and occupancy

- `OpenProjectWorkContextHandler` verifies the requested project, then creates or replaces the context identified by surface and window.
- Every open, switch, or renewal uses backend time and extends the lease by 120 seconds.
- A Tauri window cannot open a project that is actively leased by a different Tauri window. Expired leases do not block access.
- Reopening from the same window refreshes its existing stable context rather than creating a competing lease.
- `RenewProjectWorkContextHandler` fails when the surface/window context does not exist.

`ProjectWorkContextRepository` owns persistence queries for window identity, active project occupancy, updates, explicit deletion, and expired-row cleanup. The handlers own lease duration and conflict policy; transports own heartbeat scheduling and client window identity.

This module does not select projects globally for the backend, manage task worktrees, or expose these operations through every transport. See [Application and Contracts Boundary](../../../../docs/application-contracts.md) for current adapter support.

See the [ora-application overview](../../README.md).
