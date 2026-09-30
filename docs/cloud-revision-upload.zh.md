# Cloud Revision 上传契约

[English](cloud-revision-upload.md) | 中文

固定的 Cloud submodule 是协议唯一来源。生成的 v1 请求新增可选 field 3 `GrantRevisionUploadRequest.checksums`：固定对象键到小写 SHA-256 的映射。原方法和请求保持 wire 兼容，旧解码器忽略新字段。

上传方先计算对象摘要，为 Cloud 选定的对象键请求能力，PUT 时发送所有返回的签名头，其中 SHA-256 通过 `x-amz-checksum-sha256` 签名。授权过期刷新不改变执行输入和键。授权只在内存流转，不进入日志、环境、命令参数或文件；部署 S3 原始凭据不得传给 Node。

Cloud 在 SQL 事务外检查对象存在、大小和存储 SHA-256，重查租约和执行绑定，再同事务登记原 Node 证据、收据、Revision 和业务钩子。明确对象失败保留原 Node 证据并另记失败结论；网络失败不 ACK，可重放。已提交重放不再次上传或验证。

`crates/controller-proto/tests/upload_checksum.rs` 覆盖旧 wire 样本解码/原样重编码、新 map 往返与旧解码器兼容、签名头完整保留。协议 crate 测试与 clippy 通过；`ora-controller`、`ora-node` 均用新生成绑定构建。Node/process 公开协议无变化。

此配套仅同步生成协议消费。生产 Rust Controller/Node 交付 relay、授权刷新、日志/ACK 和重启验收仍属于 C/D。真实 PostgreSQL/Git/RustFS 的交付验收目前使用 Cloud Go Controller/Node 替身，证明 A，不代表生产 C/D 或 B 界面已完成。
