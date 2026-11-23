# 仓库 clone

[English](repository-clone.md) | 中文

Node 从平台 Git Server 上的快照把 Workspace 的代码 clone 到它分配的目录中，并以快照的基础提交作为 checkout 的 commit
报告。clone 是独立的执行族，有自己的记录；它不需要已有的 Main Workspace。相关决策是
[clone 根决策](https://github.com/ora-space/specs/blob/main/decisions/node/repository/0-clone-selected-repository-branch.md)、
[最小 clone 执行契约](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20260918-minimal-clone-execution-contract.md)、
[被终止的尝试以 interrupted 失败](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20260925-terminated-clone-attempt-fails-as-interrupted.md)
和[从 Git Server 快照 clone 与 ora-git](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20261016-clone-from-git-server-snapshot-and-ora-git.md)
（`approved`，端到端验收通过后改为 `implemented`）。
会话期间 Agent 经 [ora-git](ora-git.zh.md) 访问 Git Server。

## 输入

Cloud 在交出 Workspace 的 clone 步骤之前，根据 Binding 快照冻结 clone 输入，Controller 原样转交（见
[Controller](../controller/controller.zh.md#workspace-沙盒)）。`clone_repository` 携带 `node_id`、`repository_url`、
`snapshot_ref`、`base_commit` 和 `workspace_ref`：

- `repository_url` 是不含 userinfo、query 和 fragment 的 `https://` URL；
- `snapshot_ref` 指明规范的 Binding UUID，并且恰好指明 `base_commit`；
- `workspace_ref` 形如 `refs/heads/workspaces/<workspaceId>/<branch>`，带规范的 Workspace UUID 和具体分支（不能是 `HEAD`）。

旧的 `repository`/`branch` 输入和 SSH 来源都会被拒绝。见 [Controller–Node 协议](protocol.zh.md#消息契约)。

## 部署

```json
{
  "clone": {
    "repository_root": "/var/lib/ora/repositories",
    "git_config": "/etc/ora/clone.gitconfig",
    "search_path": ["/usr/bin"]
  },
  "git_access": {
    "ca_cert": "/run/ora/git-ca.pem",
    "client_cert": "/run/ora/git-client.pem",
    "client_key": "/run/ora/git-client-key.pem"
  }
}
```

- 只有同时配置了 `clone` 和 `git_access`，Node 才声明 `repository_clone`；Cloud 模式（设置了 `control.target`）的 Node
  没有 `git_access` 时以 `git_access_required` 拒绝启动。git-access 材料见 [Revision](revisions.zh.md#git-server-访问材料)。
- 根目录和配置在启动前准备好；Node 从不创建它们，也不修改它们的权限。每个路径都是绝对路径，属于当前用户或 root，不含
  符号链接组件；release 构建拒绝组或其他用户可写的路径。根目录不能与 Node 状态、配置的旧字段
  `process.host_directory` 或已配置的 worktree checkout 重叠。
- 文件系统必须报告创建时间：设备号、inode 和创建时间标识根目录以及每个新建的叶子目录。缺失或被替换的归属证据保持
  `Unknown`；仅凭数据库预留永远不能拥有一个目录。
- `git_config` 只能选择不含秘密的 CA 信任，见 [运行时控制](../controller/runtime-control.zh.md)。没有 SSH 配置：clone
  只经 HTTPS 来自 Git Server，每个 clone 命令都禁用 SSH。
- `process.command_timeout_ms` 限制本地 Git 命令；网络操作使用 `process.network_timeout_ms`（默认 600000，允许
  1..1200000）。预算耗尽时杀掉这条命令的进程组，并且要先取得这次尝试的停止证据，clone 才能报告 interrupted。

嵌入方使用 `configure_clone`、`submit_clone` 和 `recover_clones`；可执行文件在启动时恢复已受理的 clone，新的 clone 只从
控制会话接收。

## 执行

每个受理的请求拥有一个新的、独占创建的目录。clone 分两半：

1. **以 Node 身份拉取快照。** Node 只把 `snapshot_ref` 拉进 `<repository_root>/.ora-transport/` 下私有的 bare 传输仓库。
   git-access 证书、私钥、CA 以及 clone 执行的 `Ora-Execution-Id` 请求头只作为这条命令的参数传入；credential helper、
   重定向、hook 和环境中的配置都被禁用。拉到的提交必须等于 `base_commit`。传输仓库不含任何凭据，随后交给工作负载用户。
2. **以工作负载用户身份建立 checkout。** 三条已记录的工作负载命令依次初始化 checkout、把传输仓库的对象取入以
   `workspace_ref` 分支短名命名的本地分支（只有这次本地 fetch 允许 file 传输）、重置工作区。Git 运行时不使用模板、不递归
   子模块、不运行 LFS smudge 或 process 过滤器；每条命令都禁用 hook 和交互式提示。

checkout 有完整历史和独立的对象，没有 remote、没有凭据配置，也没有跟踪 ref。校验检查目录身份、普通的非浅仓库、分支、
已跟踪文件干净，以及 HEAD 等于 `base_commit`。

传输仓库用完即删，启动时也会清理；例外是工作负载命令序列尚未完成读取它那一步的 clone：那一步之前不会再拉一次快照，
它的 Git 命令可能仍在读取传输仓库。

## 失败与恢复

拉取快照只改动私有的传输仓库，所以只要上一个 Node 进程的未记录命令不再占着尝试锁，重启后就可以直接重新开始。段内重试
与重启后的重新拉取共用一个持久的累计次数：每次拉取开始前先把它加一，一共失败 3 次后 clone 以 `source_unavailable`
结束。服务器不可达和 5xx 失败先后间隔 5 秒、30 秒重试；重启后的下一次拉取立即开始，不再等待剩余的间隔。401、403 和
ref 不存在不重试。

| 代码 | 含义 |
|---|---|
| `snapshot_unavailable` | Git Server 表示快照 ref 不存在或不可读（两者回答相同），或不接受 Node 的 git-access 身份（401） |
| `base_commit_mismatch` | 拉到的 ref 不是提交，或不是 `base_commit` |
| `source_unavailable` | Git Server 一直不可达或返回 5xx，直到 3 次尝试用尽 |
| `destination_conflict`、`operation_failed`、`interrupted` | 同以往：目标冲突、本地失败、有停止证据的被终止或已停止的尝试 |

每条工作负载命令在派发前连同它的尝试锁一起记录，它的结束（Git 退出、停止宽限期或截止时间之后的终止，或重启后的收口）
只按停止证据分类，见 [Node](node.zh.md#执行与恢复)：

- 有退出码且有停止证据：继续下一步或进入校验；
- 被信号终止且有停止证据：以 `interrupted` 失败并保留目录；
- 重启后的 Node 发现尝试已停止、没有退出状态：同样以 `interrupted` 失败并保留目录；
- 缺少停止证据：保持 `Unknown`。

缺少进程证据从不成为再起一次尝试的理由。已知的失败保留其目录；重试需要新的 operation 和 execution 身份，并得到不同的
目录。终态重放始终使用原来持久化的结果和事件，不重新检查，也不重新拉取。

## 记录

clone 记录存放在 Node 数据库中：

- `accept_clone` 原子地保存原始命令、生成的仓库身份、根目录和路径以及初始预留；它从不创建目录。重放相同输入返回原目标；
  输入改变或跨执行族复用都会失败。目标不能与任何活动的 worktree 目标或保留的 clone 目标重叠，失败也从不释放其预留。
- 进度记录 `Reserved`、`DirectoryCreated`、`Fetching`（带持久的累计拉取次数）或 `Dispatched`（带当前工作负载步骤）。目录身份在
  创建后冻结，`Unknown` 保持其阶段。每个工作负载步骤最多记录一次尝试，并且之前的尝试必须先收口。
- 终态结果和它的事件在同一事务中提交，并且必须与原始输入、资源和路径一致。成功要求每个工作负载步骤都以零退出；信号终止
  和没有退出状态的停止只能支撑 `interrupted`。
- schema v10 增加 `workspace_branch_heads`：checkout 已知的、自己的 Workspace 分支在 Git Server 上的最新值，
  [ora-git](ora-git.zh.md#推送) 推送时以它声明旧值。快照输入之前写下的 clone、会话和交付记录不迁移：它们的输入已无法
  解码，所属 Workspace 重新创建。
- 查询从不确认；`pending_events` 覆盖所有执行族，确认只删除投递记录，结果、输入和预留都保留。
