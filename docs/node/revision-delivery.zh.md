# Revision 交付

[English](revision-delivery.md) | 中文

Linux Node 把已结束 Agent 会话的 checkout 与会话记录保存为 Revision（能力 `revision_delivery`，
ADR `node/revision/0`）。只要部署配置含 `clone` 段 Node 就声明该能力：交付在 clone 建立的 checkout 中、
以 clone 的加固 Git 环境运行 Git。

## 受理

- 生产入口是 `controlled_deliver_revision`：运行许可须精确匹配本 Node、incarnation、operation 与
  execution，并在受理时检查。裸 `deliver_revision` 只在从未启用运行控制的 home 中经私有 IPC 受理。
- 输入在任何工作前持久化，交付直接记为已开始（`Running`）。同一输入重发只回报状态；同一身份不同输入是协议错误。
- 账本：表 `revision_deliveries`（schema v9，身份种类 `deliver_revision`），唯一终态事件在
  `revision_outbox`。交付与会话不共用表，会话恢复不会看到交付。状态查询在终态前回答 `Running`，之后回答
  已存的 `revision` 结果。

## 准备（只做一次）

1. 会话须已有终态且 actor 不再存活，否则 `failed{session_not_settled}`；checkout 须是该会话自己的 clone 且仍以交付的
   基础提交为 clone 结果，否则 `failed{checkout_unavailable}`；会话 JSONL 不存在为 `failed{history_unavailable}`。
2. 封口的 JSONL 复制到 `<home>/revision-deliveries/<sha256(execution)>/history.jsonl`。
3. 快照使用 checkout Git 目录中的临时 index（workload 身份可写，Node home 不可写）：`read-tree HEAD`、
   `add -A`、`write-tree`。tree 等于 `HEAD^{tree}` 时最终提交即 `HEAD`；否则 `commit-tree` 追加一个提交，
   author 为会话的 git 身份，committer 为 `Ora <revision@ora.invalid>`。钩子、fsmonitor 与签名均被禁用；
   真实 index、工作区文件与分支不变。`update-ref` 让 Revision ref 指向最终提交。
4. 最终提交不同于基础提交时，`bundle create <ref> ^<base>` 与 `bundle verify` 产生 `revision.bundle`，
   与会话记录放在一起。输入带 `prior_revision`（续接的会话）且最终提交等于其 `final_commit` 时，交付以该提交
   记为 `unchanged`，不生成 bundle：前序 Revision 的 bundle 已包含它。其他新提交照常相对 clone 的基础提交打
   bundle，因此也包含前序 Revision 的提交。
5. 每个对象的大小、SHA-256 与声明（`delivered` 或 `unchanged`）在**首个 PUT 之前**作为固定计划写入账本。
   此前重启则重新准备；此后重启复用同一文件与声明，不再重新快照。

## 上传

- 授权只在内存中，不写日志、不写账本。没有有效授权（尚未收到、5 秒内过期、被 403 拒绝或因重启丢失）时，
  Node 发送 `upload_grant_needed`，携带所有待上传对象的固定 SHA-256；等待期间每 2 秒重发，每次有新的
  Controller 连接时立即重发。等待期间交付保持 `Running`，没有期限。
- 每个 PUT 流式发送固定文件，原样携带授权的全部头（含 `If-None-Match: *` 与 `x-amz-checksum-sha256`），
  不跟随重定向。2xx 与 412 视为已上传，由 Cloud 核对声明；其他结果计为一次尝试，单个对象第三次失败后交付以
  `failed{upload_failed}` 结束。
- 终态 `revision_result`（序号 1）在确认前持续重放。终态持久化后删除固定文件；残留在下次启动时清理。
- 关闭、Controller 断开与生命周期取消都不写失败终态：交付在重启后继续。

## 续接前序 Revision

同一 Issue 的新运行续接该 Issue 最近的 Revision（ADR
`node/revision/20261010-restore-prior-revision-before-session`）。Node 只在同时声明 `agent_session` 与
`revision_delivery` 时声明 `revision_restore`，因为恢复在会话 checkout 中运行交付的 Git；Controller 只把带
`prior_revision` 的会话派给这样的 Node。

会话在解析 checkout、检查插件之后，把 checkout 交还工作负载用户、启动任何插件之前进行恢复。恢复失败时会话以
`agent_failed` 结束，`detail` 为 `prior_revision_unavailable` 或 `prior_revision_base_unavailable`，从未有插件
进程存在。恢复期间 Node 重启时，会话与其他会话一样以 `interrupted` 结束；下面的交付判定对 checkout 的两种状态
都给出确定结果。

### 下载

- Node 发送 `download_grant_needed`（不占序号），等待期间每 2 秒重发，每次有新的 Controller 连接时立即重发。
  回答为 `download_grant{granted}` 或 `download_grant{refused}`；拒绝立即使恢复失败。授权只在内存中、不写日志；
  只接受该会话自己 `prior_revision.bundle.key` 的授权。
- 等待授权与下载共用 5 分钟期限。bundle 以单次预签名 `GET`（头原样、不跟随重定向、错误中不含 URL）下载到
  `<home>/revision-restores/<sha256(execution)>/prior.bundle`（目录 `0700`，文件 `0600`），以声明的大小为上限，
  须与声明的大小和 SHA-256 一致。网络错误、429 与 5xx 重试 3 次；403 丢弃授权并重新申请一次；其他回答直接失败。
  恢复结束时删除该目录，Node 启动时清空根目录。

### Git 步骤

所有命令与交付 Git 相同（受 host 管理、以工作负载身份运行、禁用 hooks/fsmonitor/签名、使用 clone 的环境与协议策略），
在阻塞线程上运行：

1. bundle 头须为 v2 或 v3（只允许 `@object-format` 能力），恰好一个 head，位于 `refs/ora/revisions/` 下并指向
   `prior_revision.final_commit`；否则为 unavailable。
2. 每个前置提交（`-<oid>`）须存在（`cat-file -e <oid>^{commit}`）。缺失的以 `fetch --no-tags origin <oids>`
   补取一次。只有远端明确回答没有该对象（`not our ref`、未公布的对象、没有该远端 ref），或补取成功后仍缺失，
   才记为 `base_unavailable`；远端不可达或拒绝访问记为 `unavailable`，Cloud 会在下一次运行时重试，而不是拒绝该 Revision。
3. 已校验的 bundle 复制到 checkout 的 Git 目录（属于工作负载用户，用后删除），再 `bundle verify`、
   `fetch --no-tags <bundle> <head>:<head>`（仅这一条命令允许 file 传输），并确认最终提交可解析。
4. 在 clone 的分支上执行 `checkout --force -B <branch> <final_commit>`；`origin/<branch>` 不动，工作区与 index
   等于前序最终提交。
5. 若某个前置提交不是 `refs/remotes/origin/<branch>` 的祖先，说明远端历史被改写：在首条提问末尾追加一段固定格式
   的英文说明，写明恢复的提交、它的基础提交与 `origin/<branch>`，它因此也进入会话记录与 Thread。

## 已知限制

- Agent 插件不在 host Scope 中运行，“进程 scope 已关闭”指会话终态已写入且会话运行时已停止插件进程树。
- 交付的 Git Run 受 host 管理，但不作为业务变更记入 journal；中断的准备只会重新准备。
- 仓库配置仍可指定 `git add` 运行的 clean/smudge filter；它们以 workload 身份运行，与 Agent 本身相同。
- 永久性的 `base_unavailable` 依赖远端对“对象不存在”的措辞；措辞不同的服务器会被报告为可重试的 `unavailable`。
