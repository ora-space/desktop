# ora-reaper

`ora-reaper` is the process-cleanup sidecar shipped with Ora Desktop and with the Node.

Ora starts the sidecar before constructing the backend. Every OS process created through the
production `TokioProcessSpawner` is registered synchronously and unregistered after its direct
child exits. During an orderly shutdown, Ora asks the sidecar to terminate any remaining process
trees and waits for acknowledgement. If Ora crashes, closing the inherited IPC pipe triggers the
same cleanup without cooperation from the parent.

`ora-node` starts the same sidecar for the Git commands it runs, so that Git does not outlive a
killed Node; its Agent session processes stay out of it. A Node that cannot start the sidecar does
not start, see [Node](../../docs/node/node.md#execution-and-recovery).

The sidecar deliberately owns no application behavior, persistence, or restart policy. Its only
interface is the private, version-locked protocol implemented by `ora-process`; its parent and the
sidecar are always built and shipped from the same source revision.
