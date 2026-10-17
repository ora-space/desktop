# 经 Git 交付 Revision

[English](revision-delivery.md) | 中文

Linux Node 将结束的 Agent 会话的最终提交推到 Git Server，对象存储只接收封存的会话 JSONL。
clone 与 git-access 同时配置后支持交付；再启用 Agent 会话后支持恢复。Cloud 模式必须配置有效 git-access。

## 受理与准备

生产入口 `controlled_deliver_revision` 核对绑定 Node、进程代次、operation、execution 的许可。只有从未启用
运行时控制的私有 IPC 才接受裸交付。不可变输入包含会话、clone 执行、快照基线、本地 Revision ref、Git 仓库
URL、交付 ref、Workspace ref 和历史对象键。Workspace ref 必须等于 clone 输入；同输入重放，矛盾输入拒绝。

先在账本记录责任，再执行准备。会话必须已结束且 checkout 可用，封存 JSONL 复制到
`<home>/revision-deliveries/<sha256(execution)>/history.jsonl`。工作负载 Git 用临时 index 执行
`read-tree HEAD`、`add -A`、`write-tree`。工作区干净时最终提交就是 HEAD；剩余改动形成它的子提交，
作者为会话身份、提交者为 `Ora <revision@ora.invalid>`。禁用 hooks、fsmonitor、签名，不改真实 index、
工作区或本地分支。本地 Revision ref 保留最终提交。

网络写入前冻结最终提交、历史字节与摘要、结果声明。冻结前重启重新准备；冻结后重启复用同一计划。
最终提交等于快照基线或续接的最终提交时为 `unchanged`；每次交付仍建立自己的交付 ref 并尝试更新
Workspace 分支。不生成、上传或下载 bundle。

## 推送与上传

Node 身份的 Git 不进入 checkout。私有传输仓库只借用对象库，拒绝嵌套 alternates 或共享目录，并禁用借用
仓库的 ref 枚举。证书、私钥、CA、执行头仅随当次 HTTPS 命令传入；checkout 配置不能重定向请求或注入 helper。

1. 把冻结提交推到 `refs/ora/deliveries/` 下的 `delivery_ref`。新建与相同提交重放成功；拒绝为
   `push_rejected`；网络重试耗尽为 `push_failed`。
2. 用 Node 账本旧值声明尝试快进 `workspace_ref`。重放成功；旧值不符、拒绝或网络重试耗尽只记分类日志，
   不使交付失败。
3. 仅为历史摘要申请内存中的上传授权，按原样请求头 PUT 冻结 JSONL，不跟随重定向。2xx 与 412 进入
   Cloud 核验；三次上传失败为 `upload_failed`；403 丢弃授权并重新申请。

Git 的瞬时故障最多尝试三次，间隔 5/30 秒。交付的每次推送尝试最长 30 分钟，与 `network_timeout_ms`
无关，一分钟内低于 1 KiB/s 时中止；401/403 与 ref 更新被拒不重试。
停止或断连保留冻结计划。序号 1 的终态事件重放直到 ACK。Cloud 在事务外核验交付 ref 和历史后登记 Revision。
授权不进入账本。终态持久化后删除冻结文件，启动时清理残留。

## Agent 启动前恢复

`prior_revision` 提供 Revision 身份、最终提交、基础提交、Git URL 与交付 ref。仓库必须等于 clone 的仓库。
会话执行授权 Node 把交付 ref 拉到私有传输仓库，协商只用快照基线；拉取提交必须等于前序最终提交。
随后工作负载 Git 从本地传输仓库取入对象，把 Workspace 本地分支检出到前序最终提交，最后才启动插件。
clone 基线不变，仓库配置与 `refs/remotes/ora/**` 保持原样。

前序基线不在当前快照历史中时，首条提问追加固定提示，列出最终提交、基础提交并建议
`ora-git fetch upstream`。交付 ref 缺失或无权为 `prior_revision_base_unavailable`；瞬时拉取、核验或
检出失败为 `prior_revision_unavailable`；拉取与取入共用 5 分钟时限（含重试），到期同样为
`prior_revision_unavailable`；均在插件启动前结束会话。恢复期间 Node 停止或收到 `EndSession` 时，恢复在
下一条 Git 命令前停止，并等待正在运行的那条结束，会话（按命令的原因）只在恢复不再写 checkout 后结束。Node 重启中断会话，不恢复旧 Agent 输入。
传输仓库用完删除，启动时清理残留。

## 部署与限制

Sandbox Server 为租户、Workspace、代次签发独立 git-access 客户端证书。入口脚本将材料写进仅 root 可读的
`/run/ora` tmpfs；工作负载只有会话 socket。证书只携带身份，Git Server 按 Cloud 当前状态派生权限。

快照过滤器仍以工作负载身份运行。借用对象的解析与资源限制留待后续加固。准备过程可重放，持久边界为冻结计划，
不作为业务 mutation journal 记录。参见[快照 clone 与协作](repository-clone.zh.md)。
