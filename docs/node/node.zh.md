# Node

[English](node.md) | 中文

`ora-node`（`apps/ora-node`）是在执行环境中运行工作的 Linux 可执行文件：在 Cloud 模式下，每个
Workspace 沙盒中运行一个 Node。它拥有一个显式配置的 Node 数据库，执行 clone、插件安装、Agent 会话和
Revision 交付，在重启后恢复未完成的执行，并把每一条 Git 命令作为自己的直接子进程运行，见
[执行与恢复](#执行与恢复)。它只通过[控制会话](control-session.zh.md)接受来自自己 Controller 的命令；配置文件和
stdin 都不是命令通道。

## 部署

用 `cargo build -p ora-node -p ora-reaper` 构建，然后运行 `ora-node /absolute/path/node-config.json`。
最小配置：

```json
{
  "node": { "home_directory": "/home/alice/.ora/node", "identity": "Discover", "repositories": [] },
  "process": {
    "git_program": "/usr/bin/git",
    "environment": { "PATH": "/usr/bin:/bin", "HOME": "/home/alice" },
    "command_timeout_ms": 30000,
    "cleanup_timeout_ms": 5000,
    "shutdown_grace_ms": 2000
  },
  "timezone": "Asia/Shanghai",
  "recovery_interval_ms": 1000
}
```

更多的配置段用来启用能力：`clone`（[仓库 clone](repository-clone.zh.md)）、`control`
（[控制会话](control-session.zh.md)）、`agent`（[Agent 会话](agent-sessions.zh.md)）、`plugins`
（[插件](plugins.zh.md)）和 `git_access`（[Revision](revisions.zh.md#git-server-访问材料)），
clone、Revision 交付和 [ora-git](ora-git.zh.md) 都需要后者；Cloud 模式的 Node 没有它时拒绝启动。
`process.workload_uid` 让 Git 工作负载和 Agent 以单独的用户运行。

- `process.reaper_program` 是 `ora-reaper` 可执行文件的绝对路径；没有它时，Node 使用与自己的可执行文件
  安装在同一目录的 `ora-reaper`。
- `process` 中有两个旧字段仍然接受。配置了 `host_directory`（绝对路径，不能包含 Node home）时，Node 在启动时
  删除已退役进程宿主的状态目录，但只删除含有 `host.sqlite` 且 `host.lock` 空闲的目录。`expected_uid` 被忽略。
- `"Discover"` 保留已有的 NodeId 或生成第一个；`{ "Require": "id" }` 强制使用一个已登记的身份。每次启动都
  生成新的 NodeIncarnationId。
- Node home 是绝对路径并由外部注入，从不从 `HOME` 推导（`environment` 中的 `HOME` 只用于配置 Git）。
  新目录是私有的；release 构建拒绝已存在但不私有的目录，debug 构建跳过权限位检查。错误的 owner 和穿过受信
  路径的符号链接都会被拒绝。Node home 必须在本地文件系统上，因为尝试锁放在其中。
- `node.repositories` 保存显式的 `RepositoryBinding`（仓库引用、Main Workspace 身份和路径、授权根目录、
  worktree 根目录），供 [worktree 执行](worktree-execution.zh.md)使用。不会 clone 任何东西，也不从 cwd 推断。

## 执行与恢复

Node 自己启动每一条 Git 命令，作为直接子进程，使用配置的可执行文件和环境。每条命令是自己进程组的组长；配置了
`process.workload_uid` 时，操作 checkout 的 Git 在 fork 之后、exec 之前降到这个用户。每条命令都带
`-c gc.auto=0 -c maintenance.auto=false`，所以 Git 不会分离出后台维护进程。

Node 启动时拉起一个 `ora-reaper` sidecar，并把自己运行的每条 Git 登记给它；Node 死亡时，reaper 杀掉已登记的
进程组。reaper 拉不起来，Node 就不启动；reaper 在 Node 运行期间退出时，Node 不再启动新的 Git，并以错误退出。
Agent 会话和插件进程不向 reaper 登记，见 [Agent 会话](agent-sessions.zh.md#记录顺序与崩溃)。

**尝试锁。** 每条有副作用的命令在私有目录 `<node.home_directory>/git-attempts/`（模式 `0700`，属于 Node 的
运行用户）中有一个锁文件 `<attempt-id>.lock`（模式 `0600`）。记录尝试和派发命令之前，锁文件已经存在；Node 在它
上面加独占 `flock`，Git 和它的所有后代继承这个打开的文件。**停止证据**就是 Node 能不阻塞地重新加上这把锁。锁仍
被占用，说明进程树里还有进程活着，命令没有停止；锁文件不存在或探测出错是 `Unknown`，从不当作“已停止”。锁文件
旁边的 `<attempt-id>.clue` 记着直接子进程的 pid、启动时间和本机启动标识。它不刷盘，只用来让重启后的 Node 在
核对 pid 没有被复用之后杀掉进程组。

| 命令 | 尝试锁 | 记入 Node 数据库 |
|---|---|---|
| Worktree 变更、clone 的 checkout 命令、Revision 恢复的命令 | 有 | 记，在派发之前 |
| Revision 交付、`ora-git` 的 fetch 和 push、clone 的传输仓库命令 | 有 | 不记：被中断的从头准备 |
| 只读查询 | 没有 | 不记 |

已记录的尝试不会被再次派发，所以丢失的结果从不授权一次新的尝试。未记录命令遗留的锁仍被占用时，不启动新的未记录
命令。只读查询同样在自己的进程组里运行，并向 reaper 登记。

**输出与期限。** stdin 关闭；stdout 和 stderr 读入有界内存，从不持久化。输出超过上限时命令失败，被截断、失败或
缺失的输出从不被当作完整的 Git 事实解析。`command_timeout_ms` 限制本地命令，`network_timeout_ms` 限制网络命令。
期限到期，或 Node 停止的宽限时间用完时，Node 杀掉整个进程组；直接子进程结束后再杀一次进程组，然后最多等待
`cleanup_timeout_ms` 取得停止证据。命令的结束是已退出（带退出码）、已终止（被信号）或未核实。未核实的命令让它的
执行保持 `Unknown`，保留它的占用，而不冲突的工作仍可继续。

**重启。** 进入任何业务恢复之前，Node 先收口上一个进程留下的锁：线索的身份核对一致时杀掉它指向的进程组，然后
探测锁。已记录的尝试如果锁已空闲，就以“已停止，没有退出状态”结束：clone 以 `Interrupted` 失败并保留目录（只有
新的执行可以重试），恢复的停止视为已确认，Worktree 执行接着检查它的资源（见
[Worktree 执行](worktree-execution.zh.md#恢复)）。锁仍被占用时保持 `Unknown`，锁文件保留，之后再次探测。已记录的
尝试没有锁文件时同样是 `Unknown`。

保证等级是 `BestEffort`，不是强隔离。不在保证之内的情况：后代进程关闭继承的 fd 并脱离进程组；锁文件被同一用户的
其他进程打开并占住（没有独立工作负载用户的部署）；Node 和 reaper 同时死亡后 Git 继续运行，直到 Node 重启。

`SIGTERM` 和 `SIGINT` 关闭准入，给正在运行的 Git 命令留出宽限时间（`shutdown_grace_ms`），然后杀掉它的进程组。
关闭过程收口待处理的尝试，报告失败而不是声称完成了未经证实的停止；业务对账可以留到下一次启动。

## 存储

数据库固定是 `home_directory/ora-node.sqlite3`。它保存业务执行、资源和事件记录，以及已记录的 Git 尝试；
尝试锁文件放在它旁边的 `home_directory/git-attempts/` 中。

- 打开时在同级的 `ora-node.sqlite3.lock` 上持有排他的操作系统租约，持续到数据库关闭，而不是锁数据库文件本身，
  否则在 macOS 和 Windows 上 SQLite 自己的锁会与整文件租约冲突。租约排除另一个 Node owner，但不证明较早的
  Git 进程已经停止。
- SQLite 使用默认的回滚日志和 `synchronous=FULL`。数据库的 application id 为 `0x4f52414e`。打开时先检查身份、
  完整性、精确的表和索引定义以及外键；空文件、其他数据库、不支持的版本、目录和损坏的数据库都会被拒绝，从不重建。
- 旧的精确 schema 在事务中迁移，保留执行、结果和待处理事件；旧的二进制拒绝更新的版本，而不是去读它。

| 版本 | 新增 |
|---|---|
| 1 | `node_metadata`、`executions`、`resources`、`outbox`（Worktree 执行） |
| 2 | `managed_executions`、`process_attempts`：进程关联 |
| 3 | `execution_identities`、`clone_executions`、`clone_outbox`、`process_outcomes` |
| 4 | `controller_binding`、`execution_controllers`：与部署绑定的 ControllerId |
| 5 | `process_terminations`：在确认清理之后被信号结束的尝试 |
| 6 | 运行时绑定、执行状态和执行许可 |
| 7 | `plugin_executions`、`plugin_outbox` |
| 8 | `node_executions`、`execution_events`、`session_commands`：Agent 会话 |
| 9 | `revision_deliveries`、`revision_outbox` |
| 10 | `workspace_branch_heads`：`ora-git` 推送时声明的自己 Workspace 分支的值 |
| 11 | `session_restores`、`restore_process_attempts`：恢复阶段、原进程身份与停止确认时钟 |
| 12 | `scheduled_artifact_deliveries`：定时产物上传；终态事件 outbox 覆盖所有执行族 |
| 13 | `process_stops`：已停止但没有退出状态的尝试，与 `process_outcomes`、`process_terminations` 互斥 |

所有执行族共享 `execution_identities`，防止不同族之间的 key 冲突。完成时，资源事实、终止结果和它的原始事件
一起提交；状态读取从不确认。确认必须匹配 Node、operation、execution 和序号，并且只删除投递记录，结果和去重
信息保留下来。迁移从不为旧的直接 Git 执行编造进程证据，所以这样的进行中执行会保持阻塞。升级到版本 13 时，
已退役的进程宿主留下的每个未收尾尝试都按“已停止，没有退出状态”收口，处于同样状态的恢复尝试视为已确认停止：
部署在这个 Node 启动之前已经结束旧的 Node、宿主和 guardian 进程。存储的尝试只保留命令的目录和参数，从不保留
它的环境，环境中可能有部署机密。

`WriteGuard` 为真实 SQLite 故障测试暴露事务失败点；`cargo test -p ora-node-db -p ora-node` 会运行它们。

恢复的 Git 准入阶段与原命令意图在同一事务记录。停止确认前不能提交会话终态；上限耗尽只保留命名残留，不删除原身份。升级前缺少关联、且尚未完成的恢复保持 `legacy_unknown`，继续拦截 checkout 与运行时准入；已完成的历史终态只作为历史保留，不再拦截后续准入。见 [Revision 恢复](revisions.zh.md)。
