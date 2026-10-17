# Agent 配置声明编译器

[English](agent-configuration-declarations.md) | 中文

`ora-plugin-config::compile_agent_configuration` 校验不可变 Agent 声明，不读取用户配置、启动进程
或发送网络请求。这是独立编译入口：现有安装配置分发器和配置存储暂不接受该形态。编译成功不等于
安装成功、获得授权或运行就绪。

## 输入

```json
{
  "schemaVersion": 2,
  "agent": {
    "modelAccess": {
      "contractVersion": 1,
      "protocols": ["anthropic-messages"],
      "modelCardinality": "single"
    }
  },
  "settings": {
    "verbose": {
      "type": "boolean",
      "title": "Verbose output",
      "description": "Include additional output detail",
      "default": false
    }
  }
}
```

- `schemaVersion` 必须为 `2`，模型访问的 `contractVersion` 必须为 `1`。
- 协议列表非空且不重复，精确名称为 `openai-completions`、`anthropic-messages`。`openai` 这类
  provider 名称和 `openai-responses` 等未支持协议会被拒绝，不会被归一为另一协议。
- `modelCardinality` 必须为 `single`，声明不能授予第二个模型的调用权。
- `settings` 可以省略；出现时必须是非空的现有标量 Setting 声明对象，显式 `null` 不算省略。
  类型仍为 string、number、boolean。
- 连接选择、API Key、运行认证头和上游 URL 不属于 `agent.modelAccess`，各层未知成员均拒绝。
  Agent 声明不能混入 MCP `transport` 或 Hook contribution。
- 复用源文件 256 KiB、最多 128 个 Setting、Setting ID、文本长度、必填标志、顺序及默认值类型
  规则。重复 JSON 键在反序列化折叠之前就被拒绝。

这里的 Settings 是作者声明的元数据，不是秘密存储或运行表达式语言。编译器不推断任意文本是否为
凭据；凭据交付及受支持非秘密选项的解释属于另行授权的消费端。

## 编译结果、身份与错误

结果包含定型模型能力与确定排序的 Setting 声明。`fingerprint` 是原始输入字节的 SHA-256 小写
十六进制；空白变化也改变身份，发布副本与包核对前不能重新序列化。既有 v1 声明指纹保持原来的
规范化语义。

错误区分超限、非法 JSON、重复键、结构/Settings 无效、schema/contract 版本不支持，以及协议
列表为空、重复或未支持。错误文本不包含被拒字符串或源片段。编译不证明来源 namespace、发行
版本/target、包摘要、包内路径、个人归属或实际模型访问能力。

## 验证

[共享样例](../crates/plugin-config/tests/fixtures/agent-configuration-contract.json) 保存字面的
`source` 字符串以及编译结果或错误分类，LF/CRLF checkout 不改变 source 字节。Rust 测试调用公开
编译入口，另覆盖原始字节和 Setting 数量边界、嵌套重复键、非法 UTF-8、安全诊断、旧指纹语义，
以及旧安装配置分发器继续拒绝新版形态。

该 crate 的验证入口为 `cargo test -p ora-plugin-config --locked` 和
`cargo clippy -p ora-plugin-config --locked -- -D warnings`。
