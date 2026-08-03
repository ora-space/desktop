## Purpose

Define the generated frontend SDK contract exported from Rust-owned endpoint metadata and TypeScript DTOs.

## Requirements

### Requirement: Frontend contract SDK SHALL be generated from Rust-owned endpoint metadata
The system SHALL define frontend-facing endpoint metadata in `ora-contracts` for each exported HTTP operation, and the repository SHALL generate a TypeScript endpoint manifest in `packages/contracts` from that Rust-owned source instead of hand-maintained frontend route definitions. Each endpoint definition SHALL include the operation name, HTTP method, full path template, request contract type, response contract type, path parameter field names, and whether the operation serializes a JSON body.

#### Scenario: Contract export generates endpoint metadata
- **WHEN** the contract export workflow runs for the current HTTP CRUD surface
- **THEN** `packages/contracts` receives generated endpoint metadata that matches the Rust-owned operation names, methods, paths, and request assembly rules

#### Scenario: Frontend code needs route metadata
- **WHEN** frontend code consumes the generated contract package
- **THEN** it reads endpoint metadata derived from `ora-contracts` instead of duplicating API methods and path templates locally

### Requirement: Generated SDK SHALL expose a runtime-agnostic typed client
The system SHALL generate a typed client in `@ora/contracts` that accepts an injected transport implementation, builds request URLs from declared path parameters, serializes JSON bodies when required, and returns typed success payloads for the exported operations without hard-coding a browser runtime.

#### Scenario: Caller executes an SDK operation with a custom transport
- **WHEN** a frontend or test caller constructs the generated client with a transport implementation
- **THEN** the caller can invoke a typed operation and the SDK builds the request from endpoint metadata before delegating execution to the injected transport

#### Scenario: Operation has both path parameters and a JSON body
- **WHEN** a generated client operation targets an endpoint whose request DTO includes path-bound fields plus body fields
- **THEN** the SDK interpolates the declared path fields into the URL and serializes the remaining body payload as JSON according to the endpoint metadata

### Requirement: Browser fetch integration SHALL be provided as a separate transport entrypoint
The system SHALL expose a browser-friendly transport from `@ora/contracts/fetch` that resolves generated endpoint paths against a server base URL, executes requests with `fetch`, and normalizes the shared web-server JSON error envelope into the SDK's transport error shape. The transport SHALL treat `baseUrl` as the server base rather than as a pre-expanded API prefix.

#### Scenario: Browser transport resolves relative server base
- **WHEN** a caller configures the fetch transport with an empty `baseUrl`
- **THEN** an endpoint path such as `/api/projects` is requested relative to the current origin without requiring the caller to repeat the API prefix

#### Scenario: Browser transport resolves absolute server base
- **WHEN** a caller configures the fetch transport with `baseUrl` set to an absolute server origin
- **THEN** the transport resolves endpoint paths against that server base and requests the full URL

#### Scenario: Server returns a structured error payload
- **WHEN** the browser transport receives a non-success HTTP response using the shared server JSON error envelope
- **THEN** it decodes that envelope into the SDK's normalized transport error shape instead of exposing raw `fetch` response handling to every caller

### Requirement: Contract and SDK export SHALL run through a canonical xtask workflow
The system SHALL provide a canonical `cargo xtask export-contracts` workflow that generates the existing TypeScript DTO outputs together with the endpoint manifest and typed SDK artifacts in `packages/contracts`. Repository convenience wrappers MAY delegate to that command, but test execution MUST NOT be the canonical contract generation path.

#### Scenario: Contributor regenerates contract artifacts
- **WHEN** a contributor runs `cargo xtask export-contracts`
- **THEN** the repository regenerates DTO files and SDK files in the expected `packages/contracts` output layout

#### Scenario: Repository task wrapper triggers export
- **WHEN** a workspace helper command wraps contract generation
- **THEN** it delegates to `cargo xtask export-contracts` rather than reintroducing a separate test-driven generator path

### Requirement: Generated SDK SHALL omit standalone worktree operations
The generated frontend SDK SHALL only expose supported public operations. Because task worktrees are backend-owned internal state, endpoint metadata and generated TypeScript client helpers SHALL omit standalone create, get, list, update, and delete worktree operations.

#### Scenario: Contract export generates endpoint metadata after the cleanup
- **WHEN** the contract export workflow runs after public worktree removal
- **THEN** the generated endpoint manifest excludes `/api/worktrees` and `/api/worktrees/{worktreeId}` operations

#### Scenario: Frontend code imports the generated client
- **WHEN** frontend code consumes `@ora/contracts`
- **THEN** it can call supported project, task, and session operations, and it cannot call generated standalone worktree client methods
