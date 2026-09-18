# Controller–Node validation evidence

English | [中文](controller-node-validation.zh.md)

The public-codec integration suite enters through `crates/node-protocol/tests/protocol.rs`.
`protocol/session.rs`, `worktree.rs`, and `execution.rs` own their fixtures and constraints;
`support.rs` shares framing and assertions, and `framing.rs` owns transport-independent frame tests.
The larger Worktree and execution field tables live in their respective `rejections.rs` submodules.
Receive counterexamples start with independent JSON and manual framing, never the public writer.
Semantic counterexamples also deserialize into invalid typed messages and exercise the writer,
checking the exact same error and empty output.

## Clone result extension (2026-09-18)

The suite now has 36 tests. Clone requests live in `repository.rs`; `repository_results.rs` owns
success/failure fixtures through both `clone_result` events and Completed queries, including retained
and no-owned-directory failures. Tests cover exact wire/fragmented I/O, input/result Node mismatch,
historical incarnations, destination/commit validation, forbidden raw diagnostics and disjoint Worktree tags.
`session::accepts_independent_and_combined_execution_capabilities` covers clone-only and combined
declarations plus duplicate/unknown rejection.

Worktree Completed fixtures and field tables moved into `worktree/completed/`.
`execution/` now tests only common commands and nonterminal state structure; the shared historical
reporter assertion is invoked by both businesses. Worktree persisted encodings and database schema
are unchanged. These tests do not prove clone execution, session capability enforcement or Controller takeover.

## Evidence and migration from the 17-test baseline

| Guarantee / former coverage                                                                                 | Current evidence                                                                                                                                                    | Status                        |
| ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| Both global round-trip tests                                                                                | `session::round_trips_messages`, `worktree::round_trips_messages`, `execution::round_trips_messages`; whole-message equality through a three-byte duplex buffer     | Covered                       |
| Hello-only fixed envelope sample                                                                            | Each business's `preserves_wire_shapes`: independent JSON versus writer output, and manually framed JSON versus complete typed reader output                        | Expanded                      |
| Clean EOF, truncated header/payload, zero/oversized length, unknown type, malformed JSON and oversized send | All eight tests in `framing.rs`, including zero output on oversized send                                                                                            | Retained                      |
| Global missing/empty string scan                                                                            | Each business's `rejects_missing_and_empty_fields`, with separate explicit required-path and nonempty-string tables                                                 | Migrated, no schema inference |
| Global versions and handshake matrix                                                                        | Each business's `rejects_versions_directions_and_payloads`; `session::rejects_inconsistent_handshakes` uses named Hello and HelloAccepted fixtures                  | Retained                      |
| Opposite directions, mismatched payload, missing type/version/payload/sequence                              | Business envelope checks and required-field tables cover every message shape in both peer directions                                                                | Retained                      |
| Unknown state, missing Completed result, result on nonterminal state, incomplete terminal payload           | `execution::rejects_structural_state_contradictions`                                                                                                                | Retained                      |
| Four Completed variants retain history and reject another Node                                              | `worktree::completed::preserves_completed_worktree_contracts`: independent wire and typed fixtures, fragmented round-trip, exact receive/send error and zero writes | Retained                      |
| Transparent operation identities                                                                            | `execution::serializes_identity_consistently_across_messages`                                                                                                       | Retained                      |
| Accepted opaque strings and unknown extension fields                                                        | Field tests pad every opaque string and compare complete wire/typed messages; wire tests add unknown envelope and payload fields and compare decoded messages       | Added direct evidence         |

### Field coverage inventory

Each row identifies the replacement for the old global fixture and all its scanned opaque fields.
Every required table also names `message_type`, `protocol_version` and `payload`. Missing fields
produce `DecodeJson`; empty and whitespace-only strings produce the exact `EmptyField` on both
codec paths. Each baseline is accepted before mutation. Replacement pointers must exist and deletion
must actually remove a field.

| Business fixtures                                            | Required fields beyond envelope                                                                                                                          | Nonempty values / special rules                                                                             |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Session Hello                                                | `controller_id`, `supported_versions`                                                                                                                    | Controller identity; empty, duplicate and unadvertised version lists                                        |
| Session HelloAccepted                                        | `selected_version`, Node identity pair, `capabilities`                                                                                                   | Node identity pair; selected/envelope version mismatch, missing/duplicate capability                        |
| Session Heartbeat                                            | Node identity pair                                                                                                                                       | NodeId and incarnation                                                                                      |
| Worktree Ensure / Remove                                     | operation/execution IDs; spec NodeId, WorkspaceId, WorktreeId, repository, Main Workspace ID/path, base ref, expected branch, path-policy kind/directory | Every listed string; optional request ID must be nonempty when present                                      |
| Worktree Ready                                               | operation/execution IDs, sequence; result Node pair, WorkspaceId, WorktreeId, facts path/branch/base commit                                              | Every listed string and optional request ID                                                                 |
| Worktree Failed / RemovalFailed                              | operation/execution IDs, sequence; result Node pair, WorkspaceId, WorktreeId, failure code/message                                                       | Every listed string except the enum code, plus optional request ID                                          |
| Worktree Removed                                             | operation/execution IDs, sequence; result Node pair, WorkspaceId, WorktreeId, outcome                                                                    | Every listed string except the enum outcome, plus optional request ID                                       |
| Execution GetStatus / EventAck                               | operation/execution IDs, target NodeId; EventAck also requires sequence                                                                                  | All three IDs                                                                                               |
| Execution Unknown / Accepted / Running                       | operation/execution IDs, reporter Node pair, state tag                                                                                                   | All four IDs; nonterminal states reject a result                                                            |
| Execution Completed Ready / Failed / Removed / RemovalFailed | status fields plus result kind and the corresponding complete Worktree result fields above                                                               | All outer and nested opaque strings; reporter/result persistent NodeIds must agree, incarnations may differ |

The historical baseline fixed wire fixtures covered all 12 message variants, all six Worktree messages with and without
`request_id`, Unknown/Accepted/Running, Completed with each of the four terminal variants, and
Removed/AlreadyAbsent removal outcomes. Expected JSON uses literal objects, not serialization of
spec or result types. JSON object order is not contractual. Adding messages or fields requires
reviewing the owning business's wire fixtures and explicit field tables; tables do not discover
new schema fields automatically. A migration audit matched all 175 opaque-field occurrences across
the 25 legal fixtures against the former scanner’s coverage, including optional request IDs.

## Validation and limits

Independent wire tests passed on the unchanged implementation alongside the original 17 tests
(20 tests at that stage). After the original migration and production refactoring, all 24 integration tests passed.
`cargo test -p ora-node-protocol` and
`cargo clippy -p ora-node-protocol --all-targets -- -D warnings` pass.

Repository checks (2026-09-10): `task format`, Rust workspace lint/tests, Tauri lint/tests, and
Desktop E2E lint/tests passed. The initial `task test` stopped on app-shell's transcript context-menu
copy test (`chat-view.test.tsx:2235`, clipboard mock not called). That test passed in isolation and
all 1,317 app-shell tests passed on a full package rerun. The remaining plugin SDK suite also passed;
all components of `task test` were therefore checked, but its initial combined invocation failed.

The suite checks message legality, not every malformed JSON document. Identically shaped create
and remove payloads remain compatible. Constructors and direct Serde conversion still do not
perform semantic validation. JSON serialization failure is not induced because current typed values
have no fallible custom serializer. Arbitrary I/O failures and atomic rollback after partial writes
are not proved by zero-write validation checks; no exhaustive mutation-testing claim is made.

Session binding, negotiated-session version enforcement, dispatch authorization, durable deduplication,
persist-before-side-effect, persist-before-ack, replay and crash recovery remain Node, Controller and
session responsibilities. These tests provide no evidence for them. Protocol logging remains deferred
under `todo-87602f0b`; source TODO coverage is documented separately.
