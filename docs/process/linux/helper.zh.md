# Linux 进程 Helper 部署与管理

[English](helper.md) | 中文

独立特权 helper 方向已记录在
[Linux 后续 ADR](../../../specs/decisions/node/process/containment/linux/20260917-independent-privileged-helper.md)。
当前可执行文件支持部署预检和**带认证的只读检查监听服务**，没有启动 API、服务安装器或 guardian 接入，
不宣称强纳管能力。

此路线暂存于当前阶段，开发优先推进[完全无需 root 的尽力清理 adapter](rootless.zh.md)。
既有 helper 代码保留；无特权 adapter 不要求部署此服务。

## 构建与配置

使用 `cargo build -p ora-process-helper` 构建。部署管理员可显式以独立 root 进程运行
`ora-process-helper --check /etc/ora/process-helper.json`，不要安装为 setuid 程序。
`--check` 和 `--serve` 都不修改 cgroup 成员；实现不会自动安装二进制、创建账号或启动特权服务。

版本 1 配置如下，数字身份仅为示例，不是默认值：

```json
{
  "version": 1,
  "manager_uid": 1000,
  "workload_uid": 2000,
  "workload_gid": 2000,
  "cgroup_root": "/sys/fs/cgroup/ora-workloads"
}
```

管理与业务 UID 必须非零且不同，业务 GID 必须非零；三者均拒绝 `4294967295`，避免凭据系统调用将其解释为“保持不变”。
拒绝未知字段和版本。配置限制为 16 KiB，
必须是 root 控制的普通文件；各级祖先也必须由 root 控制，禁止组／其他用户写入，禁止符号链接。
共享的 `ora-utils::path::open_trusted_path` 使用文件描述符固定逐级验证过的目录和目标，
不采用检查路径后再跟随可能被替换的链接的方式。

Cgroup 根必须已存在、由 root 控制，实际位于 cgroup v2 文件系统，类型为 `domain`，
没有直接附着的进程，并提供 `cgroup.events` 与可写的 `cgroup.kill`。预检不会写入控制文件。
Helper 本身必须位于工作负载子树外。检查成功只证明这些前提，不证明全部后代已退出、
不冻结后续权限、不验证账号配置，也不授予启动资格。

## 检查服务

管理员可显式运行 `ora-process-helper --serve /etc/ora/process-helper.json /run/ora-helper/control.sock`。
父目录必须预先存在，并满足相同的 root 控制、禁止其他身份写入、无符号链接检查；还须允许管理身份遍历。
端点归 `manager_uid` 所有，权限为 `0600`，不替换已有文件或 socket。SIGINT／SIGTERM 会关闭监听、
取消未完成请求，并仅删除本服务创建的 socket inode。崩溃遗留端点需要管理员检查并移除后才能重启；
自动恢复尚未实现。

每个连接在读取请求前使用内核提供的连接对端 UID 认证。管理进程不得向业务转交已认证 socket：
这是连接时身份，不是逐消息重新认证。管理身份只能查询可用状态，不能指定命令、UID、PID 或 cgroup 目标。
协议类型归 `ora-process-protocol` 所有。

每连接一问一答：四字节大端长度，随后是 UTF-8 JSON。
请求为 `{"version":1,"operation":"inspect"}`，回复为 `{"version":1,"status":"launch_unavailable"}`。
其他状态为 `unauthorized`、`invalid_request`、`unsupported_version`，拒绝未知字段。
请求在分配内存前限制为 16 KiB；已接受连接的完整交换（包括回复写入）共用五秒期限，截断帧和超时直接关闭连接。
最多并发处理 16 个连接，其余留在操作系统有界监听队列。这些传输限制不是业务终止策略；检查请求不重新验证 cgroup 状态。

## 底层启动门禁（未开放 IPC）

`ora_process_runtime::spawn_linux_helper_workload` 是供可信 helper 代码使用的底层能力，不是生产 Run API。
它重新加载 root 控制的配置，只接受已存在的 `cgroup_root/scope/run` 目标。不创建目录、不负责重复身份准入、
不持有日志：后续生命周期所有者必须排他地封闭／接纳启动并保留清理责任。该函数本身**不做调用去重，也不提供 Run 持久化**。

门禁检查迁移祖先的 root 控制权、无直接成员、Run 子树为空且未冻结，以及可写的真实 cgroup v2 成员文件。
Fork 后、exec 前，子进程向固定的成员文件描述符写入 `0` 完成自身入组，再建立独立 session、
为非标准输入输出描述符设置 close-on-exec、启用 `no_new_privs`、清空附加组、设置真实／有效／保存的 GID 和 UID，
并清空 permitted／effective／inheritable capabilities，最后以业务身份切换 cwd。任何一步失败均中止 exec，
缺少内核能力时拒绝启动。成员描述符会复制到大于 2 的位置，避免 helper 标准输入输出已关闭时被管道装配覆盖。
机制遵循 [Rust pre-exec 约束](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.pre_exec)
及 [cgroup v2 进程迁移规则](https://docs.kernel.org/admin-guide/cgroup-v2.html#organizing-processes-and-threads)。

可执行文件和 cwd 必须是绝对路径。只使用显式环境，不继承 helper 环境或标准输入输出；返回的 `Child` 持有三根管道。
调用方负责消费输出、关闭输入、回收直接子进程，并另外落实后代政策；函数报错不是清理完成或退休证据。
受信任管理员不得并发改变启动边界；同步 spawn 期间冻结目标可能阻塞启动。崩溃对账、启动取消和稳定交接
仍是开放启动 IPC 的前提。

### 特权验收测试

普通测试不提权，验证哨兵身份拒绝和执行前失败。`helper_launch_privileged` 提供一个显式 ignored 的测试，
仅供**管理员预先配置的可丢弃 Linux 环境**运行：需要 root、带 `cgroup.kill` 的 cgroup v2、专用可信配置，
且业务身份与 root／管理身份不同。将 `ORA_PROCESS_HELPER_TEST_CONFIG` 指向该配置后，显式执行
`cargo test -p ora-process-runtime --test helper_launch_privileged -- --ignored`。
这是管理员操作，不属于普通三平台 CI 矩阵。

测试仅在配置的测试根下新建 Scope／Run 目录，检查凭据、capabilities、禁止重新提权、实际成员归属、
兄弟／祖先迁移拒绝、描述符／环境隔离，以及降权后的 cwd 访问。清理仅终止并移除这些新目录，绝不删除配置根；
清理失败可能留下目录供人工检查。当前普通用户环境未运行此测试；编译通过不是正向验收证据。

## 待实现边界

启动门禁仍需真实特权正向验证，Run／Scope 资源所有权、描述符交接、业务身份下的工作区访问配置、
guardian 存续与 helper 恢复仍待实现。Root helper 不能成为任意 root 命令执行或任意 PID 迁移接口。

当前测试在不提权的条件下验证配置拒绝、非 cgroup 文件系统拒绝、CLI 失败和受信任路径处理。
真实 Unix socket 测试另外在普通用户 Linux 验证对端认证、严格帧格式、超时及监听退出。
Root／cgroup 正向部署和端点属主／清理测试仍需显式配置的环境。Crates CI 已覆盖 Linux、macOS、Windows 任务矩阵；
新增矩阵本身不构成纳管证明。
