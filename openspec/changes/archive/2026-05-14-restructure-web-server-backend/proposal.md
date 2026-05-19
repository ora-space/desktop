## Why

`apps/web/server` currently stops at bootstrap wiring inside a single `main.rs` file and does not run as an actual backend service. That blocks frontend and integration work because there is no HTTP runtime, no modular composition root, and no clear place to grow server concerns such as routing, state, configuration, and transport error handling.

## What Changes

- Add a real web server runtime for `ora-web-server` that can start, bind to a configured address, and serve HTTP requests.
- Split the current `src/main.rs` bootstrap code into focused modules for configuration, application state, route registration, transport handlers, and bootstrap wiring.
- Expose HTTP endpoints for the existing `project` CRUD application handlers so the server provides a usable first backend slice instead of only constructing handler objects.
- Keep the server composition transport-focused by reusing `ora-application` handlers and `ora-contracts` DTOs rather than moving use-case orchestration into the adapter.
- Preserve a simple in-memory bootstrap repository for the first runtime slice while designing the module boundaries so a future database-backed composition root can replace it cleanly.

## Capabilities

### New Capabilities
- `web-server-runtime`: Define the backend server runtime, configuration contract, route surface, and adapter behavior for the first HTTP-backed `project` CRUD slice.

### Modified Capabilities

## Impact

- Affected code: `apps/web/server` and its tests, with possible workspace dependency updates to add the HTTP runtime stack.
- Affected APIs: the `ora-web-server` executable behavior and its first public HTTP routes for project CRUD plus health-oriented bootstrap endpoints.
- Dependencies: expected addition of a Rust HTTP stack and supporting serde/runtime crates in `apps/web/server`.
- Systems: unblocks frontend integration, local manual testing, and future persistence-backed server composition without keeping all bootstrap concerns in one file.
