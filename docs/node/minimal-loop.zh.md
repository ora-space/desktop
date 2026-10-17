# Node 最小闭环：clone 指定仓库与分支

[English](minimal-loop.md) | 中文

## 当前方向

2026-09-18 起，最小闭环从“已有 Main Workspace 上创建／删除 task worktree”调整为
**clone 指定仓库的指定分支**：调用方指定仓库和分支，Controller 协调，Node 在执行环境完成 clone，
结果回到调用方可观察的状态。请求经 Cloud 进入（租户 clone API 与 Workspace 操作），Controller 主动拨出
Cloud 领取工作，本机联调在 cluster 仓库的 Compose 栈中进行。2026-09-20 起作为首个 Client 的非生产本机
minicloud 由浏览器直连 Controller 承载的接口，与 Controller 只拨出的部署方式不符，已于 2026-09-28 移除。

此文记录整体方向。协议、clone 持久记录和 Linux 受管执行 API 已实现，见
[clone 部署与恢复](repository-clone.zh.md)；端到端闭环尚未接通。

## 已有基础与差距

- [独立 Node](runtime.zh.md) 已有启动恢复、停止和显式数据目录；Linux host／guardian 可执行受管 Git。
- [Worktree 持久执行](persistence/worktree-execution.zh.md)已实现，但这是保留能力，不是新的首版目标。
- clone 已有独立协议结果、能力声明、存储及受管执行，不要求已有 Main Workspace，
  不将获取仓库伪装为 EnsureWorktree。
- [Node 本机 IPC](local-ipc.zh.md)与 [Controller 持久协调](../controller/local-runtime.zh.md)已接通，
  已有独立进程 clone、接管提交前强杀和 Ack 丢失恢复测试；Controller 云端形态经
  [Controller–Cloud 契约](../protocols/controller-cloud-contract.zh.md)接收 Cloud 受理的 clone。

保留稳定执行身份、派发前持久责任、进程恢复交接、结果可查询及持久接管后确认等可靠性原则。
信任体系和 Strong 继续延期；clone 只从 Git Server 读取，凭 Node 的 git-access 身份，不使用部署凭据。
现有 Backend、Worktree 数据与文件布局保持不变。

## 已批准边界与后续设计

| 主题     | 已批准政策／剩余工作                                                                   |
| -------- | -------------------------------------------------------------------------------------- |
| 输入     | 经 HTTPS 以 git-access 身份读取 Git Server 快照，不用 SSH 与部署凭据；结果等于基础提交 |
| 本地资源 | Node 在注入根下分配独占目标，保留失败／未知残留                                        |
| Git 范围 | 单分支完整历史并 checkout；禁用 hooks，不递归初始化 submodule 或下载 LFS               |
| 协调     | 本机 IPC、Controller 持久接管、原执行重放和经 Cloud 的受理已实现                       |

已讨论接受的 Worktree 管理命令禁用 hooks 和清理未知时的仓库级门禁尚未实现；
不能直接把其讨论结论当作当前代码行为或完整 clone 故障模型。

## 规格入口

[clone 根决策](../../specs/decisions/node/repository/0-clone-selected-repository-branch.md)与
[最小执行契约](../../specs/decisions/node/repository/20260918-minimal-clone-execution-contract.md)已于 2026-09-18
获批（approved），确认范围、输入、目录、内容和恢复政策。
核心测试已有真实 HTTPS／SSH clone、Node 强杀恢复、终态写入失败及 Controller Ack 丢失恢复证据；
经 Cloud 的端到端链路尚无自动化测试；Node／Controller 各 ADR 的故障组合缺口仍按各自核心用例记录。
不迁移用户目录，也不切换 Backend 写入入口。
