# 架构

[English](architecture.md) | 中文

Ora 是一个面向 AI Agent 的 IDE。本仓库用同一个 Cargo workspace 和同一组 TypeScript 包构建两样东西：**Desktop 应用**，用户在其中
与 Agent 一起处理本地项目；以及 **Controller 和 Node**，它们把 Ora Cloud 的工作带进沙盒。本文给出整体图景；每个部分都有自己的文档
（见[文档地图](README.zh.md)）。

## Desktop

```text
Web App Shell（packages/app-shell，React）
  │  @ora/contracts 客户端 ─ Tauri IPC（命令 + Channel）
  ▼
Desktop 宿主（apps/desktop/src-tauri）── surface：为 UI 插件隔离的 webview
  │  领域句柄
  ▼
ora-backend ─ 组合 ────────────────────────────────────────────────┐
  ├─ ora-application  用例 ── ora-db  SQLite（ora.sqlite3）          │
  ├─ ora-agent-runtime  ACP 会话 ──▶ Agent 插件 ──▶ Agent CLI        │
  ├─ 插件生命周期  Deno 插件进程、市场、pack、Hook                   │
  ├─ 工作流运行引擎、Effect、Git 清理 worker、调度器                 │
  └─ gitlancer  带类型的 Git CLI ──▶ 项目 checkout 和任务 worktree ◀─┘
ora-reaper sidecar ── 在退出或崩溃时结束每个登记过的进程树
```

- 前端只通过生成的 `@ora/contracts` 客户端与后端通信；每个 DTO 和操作都由 Rust 拥有，见[前端契约](core/frontend-contracts.zh.md)。
- `ora-backend` 在窄的句柄之后组合各领域 crate；Tauri 宿主保持很薄，见[应用边界](core/application-boundary.zh.md)和
  [Desktop 宿主](desktop/desktop-host.zh.md)。
- **项目**是一个仓库 checkout（它的 Main Workspace）；**任务**是由链接 worktree 支撑的隔离 Workspace。会话和工作流运行都在某个
  Workspace 中执行，见[任务与 worktree](workspaces/tasks-and-worktrees.zh.md)。
- 每个 Agent 都来自一个已安装的插件，由它通过 ACP 驱动 Agent CLI；Ora 自己记录每段对话，所以对话可以比服务商活得更久，也可以切换
  服务商，见 [Agent 运行时](agents/agent-runtime.zh.md)和[会话历史](agents/session-history.zh.md)。
- Skill 以 Effect 系统落地到 Workspace 中的文件的形式到达 Agent；MCP 服务器以会话输入的形式到达 Agent，见
  [Effect](agents/effects.zh.md) 和 [Session MCP](agents/session-mcp.zh.md)。
- 工作流在冻结的图上编排 Agent 节点，见[工作流](workflows/workflows.zh.md)。

## Cloud 运行时

```text
Ora Cloud ◀── mTLS gRPC，主动拨出 ── ora-controller ── Substrate effect ──▶ Sandbox Server
                                          │                                     │
                                          └── 经沙盒路由器的 WSS ──▶ 沙盒：ora-node
                                                                          ├─ Git
                                                                          ├─ ora-reaper
                                                                          └─ Agent 插件 + CLI
```

- Cloud（`ora-space/cloud` 仓库）是租户、Workspace、运行和权限的权威方。Controller 认领 Cloud 的工作，驱动沙盒的生命周期，并把
  Node 事件和 Thread 记录中继回去，自己从不决定业务状态，见 [Controller](controller/controller.zh.md) 和
  [Cloud 契约](controller/cloud-contract.zh.md)。
- 在每个沙盒中，Node 从 Git Server 快照 clone Workspace、安装插件、用与 Desktop 相同的 Agent 运行时运行 Agent 会话，
  并把 Revision 交付到 Git Server，见 [Node](node/node.zh.md)。
- Node 把每条 Git 命令作为直接子进程运行，各自一个进程组，并登记给 `ora-reaper` sidecar；有副作用的命令持有一把尝试锁，
  锁被释放就证明它已经停止，见 [Node](node/node.zh.md#执行与恢复)。
- 权限从头到尾都有栅栏：Cloud 授权，Controller 每次派发都获取新的许可，Node 隔离过期的资格，见
  [运行时控制](controller/runtime-control.zh.md)。
- Controller 也可以运行本地的 SQLite 形态，为一个 Node 协调 clone。

## 仓库布局

| 路径 | 内容 |
|---|---|
| `apps/` | 可执行文件：`desktop/src-tauri`、`ora-controller`、`ora-node`、`ora-reaper` |
| `crates/` | 共享的 Rust crate，都以 `ora-` 为前缀（domain、contracts、application、db、backend、agent-runtime、plugin-*、node-*、process、gitlancer、utils 等） |
| `packages/` | TypeScript 包：`app-shell`、`contracts`、`chat`、`editor`、`ui`、`plugin-sdk`、`workflow-runtime`、`workflow-mock` |
| `xtask/` | 契约导出、迁移协调和 Rust 大小检查 |
| `e2e/` | 使用假 ACP Agent 的 Desktop 端到端测试 |
| `docker/` | Node 和 Controller 镜像 |
| `third_party/cloud` | 固定版本的 Cloud 契约 submodule |
| `specs/` | `ora-space/specs` 的独立 checkout（ADR 和核心测试用例），被本仓库忽略 |

每个 crate 和包把内部细节写在自己的 `README.md` 中；这些文档覆盖跨模块边界的内容。
