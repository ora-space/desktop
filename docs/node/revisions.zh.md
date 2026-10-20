# Revision

[English](revisions.md) | 中文

Agent 会话结束时，Node 把它的 checkout 和历史保存为一个 **Revision**：一个 commit（以增量 bundle 的形式）加上定稿的会话
JSONL，通过 Cloud 签名的授权上传到对象存储。同一个 Issue 之后的运行可以在 Agent 启动之前恢复这个 Revision。相关决策是
[内部快照与增量 bundle 上传](https://github.com/ora-space/specs/blob/main/decisions/node/revision/0-internal-snapshot-and-incremental-bundle-upload.md)
和[会话之前恢复上一个 Revision](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261010-restore-prior-revision-before-session.md)
两份 ADR。Controller 一侧见 [Agent 中继](../controller/agent-relay.zh.md)，授权契约见
[Cloud 契约](../controller/cloud-contract.zh.md#revision-上传校验和)。

只要有 `clone` 配置段，Node 就声明 `revision_delivery`，因为交付在 clone 的 checkout 中运行 Git，使用 clone 加固过的 Git
环境；当且仅当它同时声明 `agent_session` 和 `revision_delivery` 时，才声明 `revision_restore`。

## 受理

- `controlled_deliver_revision` 是生产入口：它的运行时许可必须匹配本 Node、incarnation、operation 和 execution，在受理时
  检查。裸的 `deliver_revision` 只在从未启用过运行时控制的 home 上、通过私有 IPC 才被接受。
- 输入在任何工作之前持久化，交付记为 `Running`。重复相同的输入只报告状态；同一身份下不同的输入是协议错误。
- 交付保存在 `revision_deliveries`（schema v9），它唯一的终止事件在 `revision_outbox`，与会话分开，所以会话恢复从不会
  看到它们。

## 准备

准备只做一次：

1. 会话必须已有终止结果且没有活着的 actor（`failed{session_not_settled}`），它的 checkout 必须仍在交付的基础 commit 上
   （`failed{checkout_unavailable}`），它的 JSONL 必须存在（`failed{history_unavailable}`）。
2. 定稿的 JSONL 被复制到 `<home>/revision-deliveries/<sha256(execution)>/history.jsonl`。
3. checkout 的 Git 目录中的一个临时 index 给 worktree 做快照（`read-tree HEAD`、`add -A`、`write-tree`）。如果 tree 等于
   `HEAD^{tree}`，最终 commit 就是 `HEAD`；否则 `commit-tree` 添加一个 commit，作者是会话的 Git 身份，提交者是
   `Ora <revision@ora.invalid>`。hook、fsmonitor 和签名都被禁用，真实的 index、worktree 文件和分支都不受影响。
   `update-ref` 让 Revision ref 指向最终 commit。
4. 如果最终 commit 不同于基础 commit，`bundle create <ref> ^<base>` 和 `bundle verify` 生成 `revision.bundle`。对于恢复的
   会话，如果最终 commit 等于上一个 Revision 的最终 commit，交付就是该 commit 上的 `unchanged`，不生成 bundle；其他任何新
   commit 都相对 clone 的基础打包，所以也会带上上一个 Revision 的 commit。
5. 每个对象的大小和 SHA-256 以及声明（`delivered` 或 `unchanged`）在第一次上传之前作为冻结的计划写入账本。在此之前重启
   会重新准备；在此之后，复用同样的文件和声明。

## 上传

- 没有有效授权时（还没有、5 秒内过期、被 403 拒绝，或因重启丢失），Node 发送 `upload_grant_needed`，带上每个尚待上传
  对象的冻结 SHA-256，每 2 秒重复一次，每次新连接时也立即发送。交付保持 `Running`，没有截止时间。授权只存在于内存，从不
  记录日志。
- 每次 PUT 以流的方式发送冻结的文件，原样带上每个授权 header（包括 `If-None-Match: *` 和 `x-amz-checksum-sha256`），从不
  跟随重定向。2xx 和 412 都算上传成功；由 Cloud 校验声明。同一个对象失败三次，交付以 `failed{upload_failed}` 结束。
- 终止的 `revision_result`（序号 1）会一直重放直到被确认；它持久化后冻结的文件就被删除，遗留的文件在下次启动时清理。
- 关闭、Controller 断线和取消从不写入失败；交付在重启后继续。

## 恢复上一个 Revision

带 `prior_revision` 的会话在解析 checkout 并检查插件之后、在把 checkout 交给工作负载用户或任何插件启动之前恢复它。失败时
会话以 `agent_failed` 结束，带 `prior_revision_unavailable` 或 `prior_revision_base_unavailable`，从不启动任何插件进程。
恢复期间重启，会话以 `interrupted` 结束。

**下载。** Node 发送 `download_grant_needed`，每 2 秒以及每次新连接时重复，并且只接受针对它自己
`prior_revision.bundle.key` 的 `download_grant { granted }`；`refused` 让恢复失败。下载与下面的全部 Git 步骤（包括祖先检查）
共用一个绝对截止，默认 5 分钟，通过 Node 服务配置 `"restore": { "deadline_ms": 300000 }` 调整。该值必须是正整数，
表示有限且可表达的时长；非法配置拒绝服务启动。授权续期、重试、退避、下载完成以及等待恢复 Git 锁都不重置截止。
bundle 通过一次预签名的 `GET`（header 原样，不跟随重定向，错误中不出现 URL）下载到
`<home>/revision-restores/<sha256(execution)>/prior.bundle`（目录 `0700`，文件 `0600`），大小受声明值限制，并与声明的大小和
SHA-256 核对。网络错误、429 和 5xx 重试三次；403 丢弃授权并重新请求一次；其他情况都失败。之后删除这个目录。

**Git 步骤**与交付的 Git 运行方式相同（由宿主管理，以工作负载用户运行，禁用 hook、fsmonitor 和签名）：

1. bundle 头必须是 v2 或 v3（只有 `@object-format` 能力），恰好有一个位于 `refs/ora/revisions/` 下、指向
   `prior_revision.final_commit` 的 head。
2. 每个前置 commit 都必须存在；缺失的用 `fetch --no-tags origin <oids>` 获取一次。只有远端表示它没有这个对象，或获取之后
   commit 仍然缺失时，结果才是 `base_unavailable`；远端不可达或拒绝访问是 `unavailable`，Cloud 会在下一次运行时重试。
3. 校验过的 bundle 被复制到 checkout 的 Git 目录中，校验并获取（`fetch --no-tags <bundle> <head>:<head>`，仅这一条命令
   允许 file 传输）。
4. 在 clone 的分支上执行 `checkout --force -B <branch> <final_commit>`；`origin/<branch>` 不受影响。
5. 如果某个前置 commit 不是 `refs/remotes/origin/<branch>` 的祖先，说明远端历史被改写了，会在第一个轮次后追加一段固定的
   英文说明，指明恢复的 commit、它的基础和 `origin/<branch>`。

每条受管 Git 命令使用自身命令截止和恢复截止中更早的值。到期后不再启动新命令；进行中的 host 交换同样受这个截止约束，
不能因更长的传输超时推迟关闭请求。执行到期后的清理与原 Scope 关闭确认使用独立的清理预算。迟到的祖先检查不能把已经到期的
恢复改判成功。到期不是停止证据：未知停止证据的持久责任、交付隔离和残留证据上限仍是
[已批准的恢复截止 ADR](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261018-complete-restore-deadline-and-cancel-confirmation.md)
的独立落地义务。

## Git Server 访问材料

启用平台 Git Server 的部署会添加顶层的 `git_access` 配置段，带绝对路径的 `ca_cert`、`client_cert` 和 `client_key`，对应
一个为运行时的租户、Workspace 和代次签发的客户端证书，与 WSS 和模型访问的身份分开。入口程序把这些文件放在只有 root 可访问
的 `/run/ora` 中。Node 在启动时校验这些材料，不可用时拒绝启动；证书本身不授予任何仓库权限，服务器地址由执行输入提供。交付
和恢复仍使用对象存储：Git Server 交付路径是一个 `proposed` 状态的决策，这里没有实现。

## 限制

- Agent 插件在宿主 Scope 之外运行，所以"进程范围已关闭"表示会话的结果已写入，且会话运行时已停止它的插件进程树。
- 交付的 Git Run 由宿主管理，但不作为业务变更记入日志；被打断的准备只是重新准备一次。
- 仓库配置仍可以选择 `git add` 会运行的 clean 或 smudge 过滤器，它们和 Agent 本身一样以工作负载身份运行。
- `base_unavailable` 依赖远端对缺失对象的措辞；措辞不同的服务器会被报告为可重试的 `unavailable`。
