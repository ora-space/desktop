# Node 最小闭环：clone 指定仓库与分支

[English](minimal-loop.md) | 中文

## 当前方向

2026-09-18 起，最小闭环从“已有 Main Workspace 上创建／删除 task worktree”调整为
**clone 指定仓库的指定分支**。沿用本机 Desktop–Controller–Node 分工：调用方指定仓库和分支，
Controller 协调，Node 在执行环境完成 clone，结果回到调用方可观察的状态。

此文记录整体方向。协议、clone 持久记录和 Linux 受管执行 API 已实现，见
[clone 部署与恢复](repository-clone.zh.md)；端到端闭环尚未接通。

## 已有基础与差距

- [独立 Node](runtime.zh.md) 已有启动恢复、停止和显式数据目录；Linux host／guardian 可执行受管 Git。
- [Worktree 持久执行](persistence/worktree-execution.zh.md)已实现，但这是保留能力，不是新的首版目标。
- clone 已有独立协议结果、能力声明、存储及受管执行，不要求已有 Main Workspace，
  不将获取仓库伪装为 EnsureWorktree。
- Node 对外 IPC、Controller 持久协调及 Client 新入口尚未接通。已有测试不构成 clone 闭环验收。

保留稳定执行身份、派发前持久责任、进程恢复交接、结果可查询及持久接管后确认等可靠性原则。
信任体系和 Strong 继续延期；私有仓库访问使用 Node 可信部署提供的非交互凭据。
现有 Backend、Worktree 数据与文件布局保持不变。

## 已批准边界与后续设计

| 主题     | 已批准政策／剩余工作                                                     |
| -------- | ------------------------------------------------------------------------ |
| 输入     | HTTPS 与显式 SSH、部署凭据、明确分支；返回实际获取的 commit              |
| 本地资源 | Node 在注入根下分配独占目标，保留失败／未知残留                          |
| Git 范围 | 单分支完整历史并 checkout；禁用 hooks，不递归初始化 submodule 或下载 LFS |
| 协调     | 已实现持久原执行重放；本机 IPC、Controller 接管和 Client 入口仍待接通    |

已讨论接受的 Worktree 管理命令禁用 hooks 和清理未知时的仓库级门禁尚未实现；
不能直接把其讨论结论当作当前代码行为或完整 clone 故障模型。

## 规格入口

[clone 根决策](../../specs/decisions/node/repository/0-clone-selected-repository-branch.md)与
[最小执行契约](../../specs/decisions/node/repository/20260918-minimal-clone-execution-contract.md)已于 2026-09-18
获批（approved），确认范围、输入、目录、内容和恢复政策。
核心测试已有真实 HTTPS／SSH clone、Node 强杀恢复及终态写入失败证据；Controller／Client
验收仍缺失。不迁移用户目录、不接通 IPC，也不切换 Backend 写入入口。
