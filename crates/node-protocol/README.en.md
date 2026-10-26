# node-protocol

[中文](README.md)

`ThreadEvent` may carry a bounded assistant-text projection and truncation mark beside the original settled record. Old messages without either field remain valid. The projection does not replace the original record, Thread acknowledgement, or terminal result.

Optional Hello/HelloAccepted `thread_previews` negotiates the live path; `ThreadPreview` is never sent without it. Messages are bounded execution/version replacements with no durable event sequence or Ack. Settled `ThreadEvent.history_sequence` links stable positions; older messages remain decodable when the field is absent.

RuntimeControl capability represents user control fencing, independently of a single connection or heartbeat. RuntimeBinding separates user epoch, Controller epoch, runtime generation, Node incarnation, business operation, Node operation and execution. Unsupported cloud peers fail explicitly rather than ignoring new constraints.

## Non-acceptance proof messages

The optional Hello flag `execution_non_acceptance` opts into the capability with the same name.
Older Controllers receive no new capability name, and older Nodes receive no new commands.
`FenceUnacceptedExecution` carries original work, frozen input and instance identity;
`ExecutionNotAccepted` carries a durable absence proof without an ordinary event sequence or ACK.

## Verification boundary

Module tests cover their real protocol, recovery, Git or TLS boundary. Cloud doubles and the loopback fixture do not prove production multiplayer authorization. Use cluster Compose for acceptance. The third_party/cloud Gitlink pins the contract commit; generated output is never edited by hand. See [runtime control](../../docs/controller/runtime-control.md) for dependencies and missing evidence.
