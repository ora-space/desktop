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
   与会话记录放在一起。
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

## 已知限制

- Agent 插件不在 host Scope 中运行，“进程 scope 已关闭”指会话终态已写入且会话运行时已停止插件进程树。
- 交付的 Git Run 受 host 管理，但不作为业务变更记入 journal；中断的准备只会重新准备。
- 仓库配置仍可指定 `git add` 运行的 clean/smudge filter；它们以 workload 身份运行，与 Agent 本身相同。
