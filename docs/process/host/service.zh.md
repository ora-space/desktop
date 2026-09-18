# 可信本机进程宿主

[English](service.md) | 中文

Linux `ora-process-host` app 持有[宿主日志与协调器](storage.zh.md)，通过本机 IPC
接受持久 Run／Stop／Close 意图；原 [guardian](../guardian.zh.md) 独立于宿主及请求方 Node
连接存续。不需要 root、helper、令牌或密钥。这是同用户合作进程部署，不认证恶意本机程序。

## 部署与连接

用 `cargo build -p ora-process-host -p ora-process-guardian` 构建两个程序。guardian 可执行文件
部署在可信绝对路径下，祖先目录不能组／其他用户可写，也不能有符号链接。显式选择私有本地状态父目录，
满足宿主日志的文件系统和 socket 路径长度要求；app 不读取 HOME 选择状态，也不创建缺失的父目录。

```text
ora-process-host create /absolute/private/process-state /absolute/private/bin/ora-process-guardian
ora-process-host recover /absolute/private/process-state /absolute/private/bin/ora-process-guardian
```

`create` 要求状态目录不存在；`recover` 要求原完整日志和锁。失败不回退到创建。
程序作为独立服务在前台运行，Node 侧安装／引导和 Desktop 打包尚未接入。
SIGTERM／SIGINT 只停止 host 协调，不停止工作负载；终止工作须显式关闭 Scope 并观察关闭完成。
服务管理器／容器整组终止不等于只强杀 host。

`ora_process_client::ProcessHost::new(state_dir, expected_uid)` 连接显式目录，
`execute(HostOperation)` 提供 Inspect、CreateScope、Start、QueryRun、Stop、Close、QueryScope
及有界 Output。每次交换重新连接 socket，重连时保留原 ID。接受启动不等于执行成功，接受停止不等于
清理完成；丢弃 client 或取消等待不撤销已接受操作。同 Run 参数冲突拒绝。

## 协议与恢复边界

Host wire 版本 2 使用已有有界 MessagePack 帧编码（16 KiB、深度 16）。控制与输出分走
`host.sock`／`host-io.sock`，各有 16 个连接槽、五秒交换期限；输出每块至多 4096 字节，
guardian 仍采用有界易失捕获政策。慢输出连接不占控制槽。双方检查内核 UID，host 绑定来自自身日志，
不接受 Node 传入的宿主代次。

查询将持久 `last_observed` 事实与 `coordination` 状态分开。历史 Running 不证明仍然存活，
guardian 不可达不授权重启，也不擦除已记录结果；bootstrap、存储或传输失败均保留原身份和文件。

恢复取得原 `host.lock` 后检查端点 inode，只允许替换连接被拒绝的、已识别名称下的私有单链接 socket。
活跃监听者、普通文件、符号链接和不兼容日志均保留并拒绝。关闭不 unlink 端点；不认识这些 host
端点名称的旧程序拒绝目录，不挪作他用。

## 验证与剩余工作

构建两个 app 后运行 `cargo test -p ora-process-host --test service`。测试部署真实私有程序，
覆盖 host SIGKILL／重连、原 guardian／Run 延续、停止与关闭、Start 回复丢失、副作用不重复、
输出连接停滞、宿主竞争及外来文件保留。测试 HOME 与显式状态目录不同。工作区测试会构建两个 app。

[独立 Node](../../node/runtime.zh.md) 已接通受管 Git 和持久资源交接；Controller IPC 和 Backend
写入入口切换仍单独推进。当前不提供退休／垃圾回收、stdin、输出流订阅、特权 Strong、
非 Linux adapter、Node 认证或 Controller 租约。现有 Backend 进程消费者未改变。
