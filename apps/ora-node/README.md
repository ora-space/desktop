# ora-node

[English](README.en.md)

Rust Node 是执行程序。Cloud 网络入口要求双向 TLS、固定目标与 root 管理/UID 1000 工作负载分离；管理凭据和本地日志位于受保护路径。持久接受后、首次目录/Git 变更前分别检查资格，恢复不重新开放旧 incarnation（进程启动代次）。已有工作按原范围收尾。明文网络和全局个人 Git 凭据明确拒绝。examples/loopback-transport-fixture 仅用于真实 TLS、Git 和崩溃测试，使用注入的测试目标与绑定，不进入镜像。操作系统身份隔离由 cluster 真实沙盒验收。

## 验证与边界

声明式模型运行先校验包的管理端 digest 记录、声明、target、协议与 Settings，再通过专用 mTLS 获取授权。包记录位于 `plugins/.node-receipts`；升级前没有记录的包必须经校验重新下载，才能承接声明式运行。旧 OpenCode 绑定保留显式兼容路径。新上下文只经启动 RPC 交给插件，精确应用确认后才发首条 prompt；token 不进入控制输入、ledger 或历史，插件与子进程使用独立 HOME 和空的初始环境。Node 负责续期、撤销及清理。

模块测试覆盖真实协议、恢复日志、Git 或 TLS 的所属边界；cloud fixture 及回环程序不证明生产多人授权。真实部署用 cluster Compose，契约版本由 third_party/cloud 的 gitlink 固定，生成文件不手改。完整验收与缺口见根 [运行时控制说明](../../docs/controller/runtime-control.zh.md)。
