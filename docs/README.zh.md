# Ora Desktop 文档

[English](README.md) | 中文

这些文档跨模块边界描述 Ora Desktop 以及 Cloud 运行时的可执行文件（Controller、Node）。某个 crate 或包内部的实现细节写在
该目录的 `README.md` 中；决策的权威来源是 [ora-space/specs](https://github.com/ora-space/specs/tree/main/decisions) 中的 ADR。

## 从哪里开始

| 我想…              | 先读                                                                                                              |
| ------------------ | ----------------------------------------------------------------------------------------------------------------- |
| 了解整个系统       | [架构](architecture.zh.md)                                                                                        |
| 在自己的机器上运行 | [本地开发](development/local-development.zh.md)                                                                   |
| 添加或删除一个功能 | [修改代码](development/changing-the-code.zh.md)，再读对应领域的文档                                               |
| 运行测试           | [测试](development/testing.zh.md)                                                                                 |
| 部署 Cloud 运行时  | [容器镜像](operations/container-images.zh.md)、[Controller](controller/controller.zh.md)、[Node](node/node.zh.md) |

## 全部文档

**核心**

- [应用边界与领域模型](core/application-boundary.zh.md)：crate 归属、实体、契约形态、公开错误、处理函数
- [前端契约](core/frontend-contracts.zh.md)：端点目录、生成、带类型的客户端、传输层、错误本地化
- [前端架构](core/frontend-architecture.zh.md)：翻译、查询归属、公开的功能接口、带类型的测试传输层
- [持久化](core/persistence.zh.md)：迁移目录、启动与开发期协调、连接、仓储
- [日志](core/logging.zh.md)：配置、JSON 事件契约、请求完成、Git 命令日志
- [流与取消](core/streams.zh.md)：流注册表，以及一次取消确认了什么

**Desktop**

- [Desktop 宿主](desktop/desktop-host.zh.md)：启动、命令层、原生命令、持久化路径、更新
- [设置](desktop/settings.zh.md)：开发者模式、日志级别、网络代理、worktree 根目录

**Workspace**

- [任务与 worktree](workspaces/tasks-and-worktrees.zh.md)：Workspace、创建任务、checkout 解析、持久的 Git 清理
- [Workspace 文件](workspaces/workspace-files.zh.md)：浏览、搜索、文件操作、监听、查看器
- [Workspace 评审](workspaces/workspace-review.zh.md)：diff、暂存、提交与推送、评审刷新
- [Gitlancer](workspaces/gitlancer.zh.md)：带类型的 Git CLI 运行时

**Agent**

- [Agent 运行时](agents/agent-runtime.zh.md)：来自插件的 Agent、监督、会话、流控、限制、宿主预览
- [会话历史](agents/session-history.zh.md)：记录、切换 Agent、降级的历史、删除
- [Session MCP](agents/session-mcp.zh.md)：选择、投递、实时刷新、Host MCP 健康状态
- [Agent 定义与 Skill](agents/agents-and-skills.zh.md)：角色、Skill 目录、包与导入
- [Skill 插件配置](agents/skill-configuration.zh.md)：公开与敏感字段、启动环境快照、重启生效和安全边界
- [Effect](agents/effects.zh.md)：Workspace Effect 收敛与 Skill 落地

**插件**

- [插件系统](plugins/plugin-system.zh.md)：种类、布局、生命周期、协议与 SDK、配置、Hook
- [Agent 配置](plugins/agent-configuration.zh.md)：静态声明、模型能力、发行关联与安装核对
- [Marketplace](plugins/marketplace.zh.md)：源、同步与索引、安装、pack
- [Surface](plugins/surfaces.zh.md)：workbench 和 webview 插件界面
- [插件日志](plugins/plugin-logging.zh.md)：由宿主拥有的按插件区分的日志
- [工作流包](plugins/workflow-packages.zh.md)：从 `.orax` 包导入工作流

**工作流**

- [工作流](workflows/workflows.zh.md)：定义、版本、图存储、节点种类、不参与执行的节点
- [工作流运行](workflows/workflow-runs.zh.md)：运行、Agent 节点、变量、失败、重试、恢复与回滚
- [循环与迭代](workflows/loops-and-iterations.zh.md)：容器节点、轮次与执行范围

**定时任务**

- [定时任务](scheduled-tasks/scheduled-tasks.zh.md)：定义、授权、运行与错过的触发点

**Controller**

- [Controller](controller/controller.zh.md)：协调存储、预览能力选择、可执行文件、cloud 形态与 Workspace 沙盒
- [Agent 中继](controller/agent-relay.zh.md)：会话、Thread 事件、命令、Revision 交付与授权
- [Cloud 契约](controller/cloud-contract.zh.md)：服务、上传校验和、生成与升级契约
- [运行时控制](controller/runtime-control.zh.md)：身份、经过认证的通道、关闭与强制停止

**Node**

- [Node](node/node.zh.md)：部署、执行与恢复、存储
- [控制会话](node/control-session.zh.md)：传输、归属、关闭码、准入、存活
- [Controller–Node 协议](node/protocol.zh.md)：分帧、能力、消息契约、编解码器
- [仓库 clone](node/repository-clone.zh.md)：Git Server 快照输入、传输仓库、失败与记录
- [Worktree 执行](node/worktree-execution.zh.md)
- [Node Agent 会话](node/agent-sessions.zh.md)：每个执行一个运行时、工作负载用户、实时预览、正文投影、账本、模型访问
- [Revision](node/revisions.zh.md)：经 Git 交付、会话历史上传与恢复上一个 Revision
- [ora-git](node/ora-git.zh.md)：Agent 使用的 Git 命令、每个会话的 socket、动词、推送与退出码
- [Node 插件](node/plugins.zh.md)

**运维**

- [容器镜像](operations/container-images.zh.md)
- [沙盒路由](operations/sandbox-routing.zh.md)
- [Sidecar](operations/sidecars.zh.md)

**开发**

- [本地开发](development/local-development.zh.md)
- [测试](development/testing.zh.md)
- [修改代码](development/changing-the-code.zh.md)

## 维护规则

- `docs/` 只放与代码一起维护的文档。计划、进度、实现日志、调研笔记、事故复盘和验收记录不属于这里；它们放在跟踪这项工作的 issue 中。
- 每篇文档都有英文版（`x.md`）和中文版（`x.zh.md`），在标题下互相链接（`English | [中文](x.zh.md)` 和
  `[English](x.md) | 中文`）。在同一个改动中修改两者；英文文档链接到英文文档，中文文档链接到中文文档。
- 描述当前代码能够验证的行为。链接相关的 specs ADR，而不是复述它们；永远不要把 `proposed` 的决策，或 `approved` 但尚未实现的决策，
  描述成已经实现。
- 一个主题只放在一个地方。跨模块的行为写在这里；crate 的内部细节写在它的 `README.md` 中。
- 行为变更要在同一个改动中更新对应的文档。新主题在合适的目录中新建文档，并在这份地图中加一行。
