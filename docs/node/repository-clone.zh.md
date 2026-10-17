# 受管快照 clone 与 Workspace Git

[English](repository-clone.md) | 中文

Cloud 在交出 Workspace 的 clone 步骤前冻结 Binding 快照。Controller 原样转交 `repository_url`、
`snapshot_ref`、`base_commit`、`workspace_ref`。Node 只接受不带凭据、查询或片段的 HTTPS URL；
快照 ref 必须包含规范 Binding UUID 并指向基础提交；Workspace ref 必须包含规范 Workspace UUID
与具体分支名。旧的 repository/branch 输入和 SSH clone 被拒绝。

## 部署与执行

`clone` 配置预先存在的绝对路径 `repository_root`、受信 `git_config` 文件与 `search_path`（没有 SSH 配置：clone 只经 HTTPS 来自 Git Server）；
`git_access` 配置 `ca_cert`、`client_cert`、`client_key`。Cloud 模式必须启用 git-access；clone 能力
要求两项同时配置。clone 根目录不能与 Node、process 状态或 Worktree 重叠。目录预留由设备、inode、
创建时间的原生证据保护；未知或被替换的证据不能授权写入。

Node 在 `<repository_root>/.ora-transport/` 下的私有裸传输仓库中拉取冻结的快照。git-access 证书与
clone 执行的 `Ora-Execution-Id` 仅经调用参数传入；禁用 credential helper、重定向、hooks 与环境中的
Git 配置。拉到的提交必须等于 `base_commit`。

传输仓库不含凭据，交给工作负载用户后，由 host 管理的命令初始化 checkout、从本地传输仓库把对象取入
Workspace 分支并检出。只有本地取入命令允许 file 协议。checkout 包含完整历史、独立对象库，没有 remote、
凭据配置或远端跟踪 ref。成功前核对目录身份、分支、已跟踪文件干净且 HEAD 等于基础提交。
传输仓库用完删除，启动时清理残留；例外是工作负载序列尚未完成本地取入的 clone：这一步之前不会再拉取
快照，它的 Run 也可能仍在 guardian 下读取传输仓库。

## 恢复与失败

每条工作负载命令先持久关联再派发。命令完成后先关闭原 Scope、确认清理，再核实仓库。已确认的信号
终止报告 `interrupted`；缺少进程或目录证据时保持 Unknown 并保留目录。已知失败保留预留；业务重试使用
新的执行身份与目标目录。终态重放直接返回原结果，不重新核实或拉取。

快照拉取只修改私有传输仓库，所以重启后可以重做；持久次数跨重启最多三轮。每轮对网络与 5xx 故障
最多尝试三次，间隔 5/30 秒。快照不存在或无权为 `snapshot_unavailable`；提交不符为
`base_commit_mismatch`；网络重试耗尽为 `source_unavailable`。本地命令受 `command_timeout_ms`
约束，网络命令受 `network_timeout_ms` 约束（默认 600000，范围 1..1200000）。清理未知不能触发重派。

## Agent 协作

`agent.ora_git_directory` 指定 Node 所有的 socket 目录，通常为 `/run/ora-git`。Node 为每个会话创建随机
socket，只把 `ORA_GIT_SOCKET` 交给工作负载。请求归属由 socket 决定，对端必须是工作负载 UID。Node 停止、
收到 `EndSession`（会话尚未处理时即生效）或会话已结束后，请求返回 `no_session`。每个会话一次只运行一个
请求，并发请求返回 `busy`。会话离开运行时，socket 立即拒绝新请求，但等待 Git 仍在运行的请求结束，
因此会话记为结束、交付可能开始时，它请求的操作不再写 checkout 或 Git Server。连接在应答前被关闭时，
客户端报告 `no_session`。

镜像在工作负载 PATH 中提供 `ora-git`：

- `info` 不访问网络，显示 Workspace 身份、分支与基线。
- `list` 列出 upstream 与 Workspace ref，并附会话冻结目录中的 Issue、run 信息。
- `fetch upstream` 与 `fetch workspace <uuid> [branch]` 只更新 `refs/remotes/ora/**`。
- `push [branch]` 先在本地检查快进，再推送本 Workspace 的同名分支。

推送用 `--force-with-lease` 声明 Node 账本的已知旧值，不信任 Agent 可改的跟踪 ref。旧值不符返回
`conflict`，但远端已经是本次提交时按重放成功处理。非快进在发出 push 前拒绝。fetch 保留本地分支、HEAD、
index、文件和配置。最终授权仍由 Git Server 按存活会话执行判定。CLI 不接受 URL、refspec 或 force 参数。

参见[持久记录](persistence/repository-acquisition.zh.md)、[Revision 交付](revision-delivery.zh.md)
与[运行时所有权](runtime.zh.md)。
