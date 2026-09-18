# Process documentation

English | [中文](README.zh.md)

Implementation and deployment documentation for the process runtime. Start with the runtime status;
the component documents describe their own behavior, verification evidence and remaining limits.

| Topic                                         | Document                                    |
| --------------------------------------------- | ------------------------------------------- |
| Runtime status and ownership                  | [Runtime](runtime.md)                       |
| Trusted-local host app and IPC                | [Host service](host/service.md)             |
| Host journal, durable intent and recovery     | [Host storage](host/storage.md)             |
| Independent guardian and Run management       | [Guardian](guardian.md)                     |
| Rootless Linux best-effort tracking           | [Linux rootless adapter](linux/rootless.md) |
| Privileged Linux helper deployment and limits | [Linux helper](linux/helper.md)             |

Design decisions remain in the [process ADRs](../../specs/decisions/node/process/README.md).
Node execution persistence is documented separately under [Node persistence](../node/persistence/worktree-execution.md).
