## 1. Add server runtime foundations

- [x] 1.1 Add the HTTP runtime, router, and serialization dependencies required for `apps/web/server` to run as a backend service.
- [x] 1.2 Introduce typed web server configuration parsing for `ORA_HOST` and `ORA_PORT`, defaulting to `0.0.0.0:32578`, and extend bootstrap errors to report invalid server configuration explicitly.
- [x] 1.3 Keep logging initialization as the first bootstrap step and ensure the runtime can start only after configuration and logging setup succeed.

## 2. Split the server crate into focused modules

- [x] 2.1 Extract the in-memory bootstrap repository, clock, and project API wiring from `apps/web/server/src/main.rs` into dedicated bootstrap-oriented modules.
- [x] 2.2 Add shared application state and router-construction modules so `main.rs` becomes a thin process entry point.
- [x] 2.3 Create transport-specific error handling that maps application failures into stable HTTP responses in one central module.

## 3. Expose the first HTTP backend surface

- [x] 3.1 Add liveness and readiness HTTP endpoints that confirm process health and only report readiness after application-state bootstrap completes successfully, without invoking project use cases.
- [x] 3.2 Implement HTTP create, get, list, update, and delete project handlers that translate requests into `ora-contracts` DTOs and delegate to the existing `ora-application` handlers.
- [x] 3.3 Register the project CRUD routes and health routes in the top-level router, then start the listener with the configured bind address.

## 4. Verify behavior and document the runtime

- [x] 4.1 Add focused tests for configuration parsing, transport error mapping, and representative project and health routes.
- [x] 4.2 Update any affected `docs/` content that describes backend runtime expectations or local server usage if the new HTTP entry point changes developer workflows.
- [x] 4.3 Run `cargo fmt --all` and `task test`, then resolve any failures introduced by the new server runtime and module split.
