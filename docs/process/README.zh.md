# Process 文档

[English](README.md) | 中文

进程运行体系的实现与部署说明。先阅读运行体系状态；各组件文档分别记录自身行为、验证证据及剩余边界。

| 主题                         | 文档                                         |
| ---------------------------- | -------------------------------------------- |
| 运行体系状态与所有权         | [运行体系](runtime.zh.md)                    |
| 可信本机宿主 app 与 IPC      | [Host 服务](host/service.zh.md)              |
| 宿主日志、持久意图与恢复     | [Host 存储](host/storage.zh.md)              |
| 独立 guardian 与 Run 管理    | [Guardian](guardian.zh.md)                   |
| 无需 root 的 Linux 尽力跟踪  | [Linux 无特权 adapter](linux/rootless.zh.md) |
| Linux 特权 helper 部署与边界 | [Linux helper](linux/helper.zh.md)           |

设计决策仍位于 [process ADR](../../specs/decisions/node/process/README.md)。
Node 执行持久化另见 [Node persistence](../node/persistence/worktree-execution.zh.md)。
