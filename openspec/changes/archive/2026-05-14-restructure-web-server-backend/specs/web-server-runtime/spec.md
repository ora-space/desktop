## ADDED Requirements

### Requirement: Web server runtime SHALL bootstrap into a listening HTTP service
The system SHALL make `ora-web-server` start a real HTTP server process after logging initialization succeeds. The runtime SHALL load a typed server bind configuration from `ORA_HOST` and `ORA_PORT`, SHALL default that bind configuration to `0.0.0.0:32578`, SHALL fail startup on invalid bind configuration, and SHALL continue serving requests until shutdown.

#### Scenario: Server starts with default configuration
- **WHEN** `ora-web-server` starts without overriding its bind-related environment variables
- **THEN** it initializes logging, constructs application state, binds `0.0.0.0:32578`, and begins serving HTTP requests

#### Scenario: Server rejects invalid bind configuration
- **WHEN** `ora-web-server` receives an invalid host or port configuration value during bootstrap
- **THEN** startup fails with a typed bootstrap error instead of silently falling back to an unexpected listener address

### Requirement: Web server runtime SHALL expose operational health endpoints
The system SHALL expose lightweight HTTP endpoints that allow callers to verify process liveness and bootstrap readiness without invoking project application use cases directly. The readiness endpoint SHALL return success only after application-state bootstrap has completed successfully.

#### Scenario: Liveness endpoint is requested
- **WHEN** a caller sends an HTTP request to the configured liveness route
- **THEN** the server returns a successful response that confirms the process is running

#### Scenario: Readiness endpoint is requested after successful bootstrap
- **WHEN** a caller sends an HTTP request to the configured readiness route after application state finishes bootstrapping
- **THEN** the server returns a successful response that confirms the runtime is ready to handle requests

#### Scenario: Readiness endpoint is requested before bootstrap succeeds
- **WHEN** the runtime has not completed application-state bootstrap successfully
- **THEN** the readiness route does not return a success response

### Requirement: Web server runtime SHALL expose HTTP project CRUD routes backed by application handlers
The system SHALL expose HTTP routes for create, get, list, update, and delete project operations, and each route SHALL translate transport input into the corresponding `ora-contracts` request DTO before delegating to the matching `ora-application` project handler.

#### Scenario: Client creates a project over HTTP
- **WHEN** a caller submits a valid create-project HTTP request payload
- **THEN** the server invokes `CreateProjectHandler` and returns a serialized create-project response derived from `ora-contracts`

#### Scenario: Client lists projects over HTTP
- **WHEN** a caller requests the project listing route
- **THEN** the server invokes `ListProjectsHandler` and returns a serialized list-projects response derived from `ora-contracts`

#### Scenario: Client deletes a project over HTTP
- **WHEN** a caller requests deletion for an existing project identifier
- **THEN** the server invokes `DeleteProjectHandler` and returns the delete-project contract response rather than transport-local soft-delete details

### Requirement: Web server runtime SHALL map application failures into stable HTTP responses
The system SHALL centralize transport error mapping for `ora-web-server` so application-layer not-found and repository failure outcomes become stable HTTP error responses instead of leaking internal error formatting directly to callers.

#### Scenario: Requested project does not exist
- **WHEN** a get, update, or delete route delegates to an application handler that returns a not-found outcome
- **THEN** the server responds with an HTTP not-found status and a structured error payload

#### Scenario: Application operation fails internally
- **WHEN** a project route delegates to an application handler that returns an internal repository or bootstrap failure
- **THEN** the server responds with an HTTP server-error status and a structured error payload without exposing transport-irrelevant internals
