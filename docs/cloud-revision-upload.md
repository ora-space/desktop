# Cloud Revision upload contract

English | [中文](cloud-revision-upload.zh.md)

The pinned Cloud submodule is the single protocol source. The generated v1 request adds optional field 3, `GrantRevisionUploadRequest.checksums`: fixed object key to lowercase SHA-256. Existing requests and methods remain wire-compatible; legacy decoders ignore the new field.

The uploader computes the object checksum, requests a capability for its Cloud-chosen key, and sends every returned signed header with PUT. In particular, SHA-256 is signed as `x-amz-checksum-sha256`. Refreshing an expired grant does not change execution input or keys. Capabilities must stay in memory and must not enter journals, logs, environment, command arguments or files. Raw deployment S3 credentials are never sent to Node.

Every PUT, including a legacy grant, signs `If-None-Match: *` so an unexpired capability cannot overwrite an object after Cloud verifies it. A retry returning 412 preserves the first object; the uploader submits its original metadata declaration for Cloud verification. The status alone does not prove matching bytes. A Controller cancellation leaves the execution replayable instead of persisting `upload_failed`.

Each execution must durably freeze object bytes and metadata before its first PUT. Restart after a partial upload reuses the original snapshot and bundle with fresh grants; changed content requires a new Cloud work item and object keys. The Cloud Go doubles cover this recovery boundary; the production Rust Node implements it as described in [Revision delivery](node/revision-delivery.md).

Cloud verifies existence, size and stored SHA-256 outside SQL, rechecks the lease and live execution bindings, then atomically records Node evidence, receipt, Revision and the business hook. Definitive object failures preserve raw Node evidence with a separate failed verdict; network failure gives no ACK and permits replay. Committed replay does not upload or verify again.

`crates/controller-proto/tests/upload_checksum.rs` tests golden legacy wire decoding/re-encoding, new checksum-map round trip with an old decoder, and checksum/conditional-header preservation. The protocol crate passes tests and clippy; both `ora-controller` and `ora-node` consumers build with the generated bindings. No Node/process public protocol is changed.

This companion synchronizes generated consumption only. The production Rust Controller relays deliveries and checksum-bound grants as described in [the Agent relay](controller/agent-relay.md); the production Node delivery, its journal/log hygiene and end-to-end restart acceptance remain C/D work. Real PostgreSQL/Git/RustFS delivery acceptance currently uses Cloud's Go Controller/Node doubles and proves A, not production C/D or the B UI.

Resumed runs extend the same contract without changing published fields: `AgentSessionSpec.prior_revision` (field 6) names the prior Revision and its verified bundle object, `DeliverRevisionSpec.prior_revision` (field 7) only its final commit, and `RevisionUnchanged.final_commit` may equal that prior final commit. `AgentRunService.GrantRevisionDownload` signs a memory-only `GET` of exactly that bundle for a registered session without a result. The Controller relays it as described in [the Agent relay](controller/agent-relay.md#resumed-sessions-and-download-grants) and the Node restores as described in [Revision delivery](node/revision-delivery.md#resuming-a-prior-revision).
