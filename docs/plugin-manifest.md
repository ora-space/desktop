# Plugin Manifest 设计方案

## 1. 目标

新增独立的 `ora-plugin-manifest` crate，用于解析和校验 Ora 插件的发布 manifest。

每份 manifest 只描述一个插件的一个发布版本。该 crate 只处理调用方提供的 TOML 文本，不读取文件、不推断固定文件名，也不访问网络。

## 2. 范围

### 2.1 本次包含

- 新增目录 `crates/plugin-manifest`。
- crate 名称为 `ora-plugin-manifest`，Rust 库名为 `ora_plugin_manifest`。
- 解析 TOML 文本并生成经过语义校验的强类型领域对象。
- 提供结构化的解析错误和字段校验错误。
- 在 `ora-utils` 中新增可复用的 slug 与 Git branch name 强类型及校验能力。
- 为新增 crate 和 `ora-utils` 职责变化补充或更新英文 `README.md`。

### 2.2 本次不包含

- 不修改 `ora-plugin-manager` 当前基于 `package.json` 的已安装插件发现流程。
- 不负责插件安装、下载、SHA-256 内容校验、运行或更新。
- 不负责读取文件、选择文件名、限制文件大小、处理符号链接或附加文件路径诊断。
- 不发起网络请求，不验证下载地址、仓库或分支是否真实存在。
- 不支持 TOML 序列化、写回或保留注释和字段顺序。
- 不引入旧格式兼容层。

## 3. Manifest 格式

```toml
resolver = 1

name = "user.ora-weather"
namespace = "official"
kind = "workbench"
version = "1.2.0"
description = "获取实时天气信息的 Ora 插件"
homepage = "https://github.com/user/ora-weather"
license = "MIT"

url = "https://github.com/user/ora-weather/releases/download/v1.2.0/ora-weather-v1.2.0.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"

[head]
repository = "https://github.com/user/ora-weather.git"
branch = "main"

[dependencies]
ora = ">= 0.8.0"
```

### 3.1 字段要求

必填字段：

- `resolver`
- `name`
- `namespace`
- `kind`
- `version`
- `description`
- `url`
- `sha256`

可选字段：

- `homepage`
- `license`
- `head`
- `dependencies`

解析器必须拒绝未知字段。必填字符串不得为空，也不得通过自动 trim 被修正。

## 4. Resolver

`resolver` 是整个 manifest 格式及其解析语义的不兼容版本号，而不只是依赖求解算法的版本。

- 使用无符号整数表示。
- 当前唯一支持的值是 `1`。
- 不支持的值返回结构化错误 `UnsupportedResolver { found }`。
- 解析器不得对未知 resolver 降级处理或猜测其 schema。

## 5. 领域模型与校验

### 5.1 插件名称

`name` 本身是完整插件 ID，例如 `user.ora-weather`。它由一个或两个 slug 段组成，最多包含一个 `.`。

每个 slug 段必须匹配：

```text
[a-z0-9]+(?:-[a-z0-9]+)*
```

额外限制：

- 每段长度为 1 至 63 字节。
- 完整名称最长 128 字节。
- 只允许 ASCII 小写字母、数字和作为段内分隔符的单个连字符。
- 不允许大写字母、下划线、Unicode、连续点或首尾点。

`PluginName` 属于 `ora-plugin-manifest`。它保留完整 ID，并使用 `ora_utils::Slug` 校验点号两侧的段。

### 5.2 Namespace

`namespace` 表示插件来源分类，不参与插件 ID 的组成。

当前仅允许：

```toml
namespace = "official"
```

模型使用封闭枚举，未知值直接拒绝。

### 5.3 Kind

当前仅允许：

```toml
kind = "workbench"
```

模型使用封闭枚举 `PluginKind::Workbench`，不提供 `Other(String)`。

### 5.4 Version

- 使用 `semver::Version`。
- 支持完整 SemVer，包括 prerelease 和 build metadata。
- 不根据版本号推断或验证下载 URL 的命名。

### 5.5 Description

- 必填。
- 允许 Unicode。
- 长度为 1 至 1000 字节。
- 拒绝控制字符。
- 拒绝首尾空白，包括仅由空白组成的值。
- 不自动 trim。

### 5.6 License

- 可选。
- 暂不引入 SPDX 解析依赖，也不声称该字段已经通过 SPDX 校验。
- 存在时必须是 1 至 256 字节的非空 ASCII 文本。
- 拒绝首尾空白和控制字符。

### 5.7 URL

`url`、`homepage` 和 `head.repository` 均使用经过验证的 URL 强类型，并遵守以下共同规则：

- 只允许 HTTPS。
- 最长 2048 字节。
- 拒绝用户名和密码。
- 拒绝 fragment。
- 允许显式端口。
- 不访问网络，不验证资源是否存在。

字段差异：

- 发布包 `url` 允许 query，以支持签名下载链接。
- `homepage` 拒绝 query。
- `head.repository` 拒绝 query，不要求 `.git` 后缀，也不限定 GitHub。
- 暂不支持 SSH 或 SCP 风格的 Git 地址。

### 5.8 SHA-256

- 必须恰好是 64 个十六进制字符。
- 输入允许大小写十六进制。
- 内部使用 `[u8; 32]` 表示，避免非法摘要状态。
- 格式化输出时统一使用小写。
- manifest crate 只校验摘要格式，不下载或计算发布包摘要。

### 5.9 Head

`[head]` 整体可选。存在时：

- `repository` 必填并遵守 HTTPS repository 规则。
- `branch` 必填，拒绝空值、首尾空白和控制字符。
- `branch` 使用 `ora_utils::GitBranchName`。
- 不提供默认 branch，也不猜测 `main`。
- 不检查远端分支是否存在。

### 5.10 Dependencies

当前只允许 Ora 宿主版本约束：

```toml
[dependencies]
ora = ">= 0.8.0"
```

- 使用 `semver::VersionReq`。
- 接受该 crate 支持的完整版本约束语法，包括比较器、caret、tilde、通配符和组合约束。
- 未声明 `[dependencies]` 表示兼容要求未知，而不是兼容所有 Ora 版本。
- 空的 `[dependencies]` 表规范化为未声明依赖。
- 未知依赖键直接拒绝。

### 5.11 发布包平台语义

单个 `url + sha256` 描述一个与操作系统和 CPU 架构无关的 `.orax` 发布包。本版 schema 不提供按 target 选择 artifact 的能力。

## 6. 公共 API

主要入口：

```rust
impl PluginManifest {
    /// Parses and validates one plugin release manifest from TOML text.
    pub fn parse(source: &str) -> Result<Self, ManifestError>;
}
```

设计约束：

- 不提供接受路径的 API。
- 不在错误中保存来源路径；调用方负责附加文件来源信息。
- `PluginManifest` 及其子结构字段保持私有，通过访问器读取。
- 公开值类型提供必要的 `Debug`、`Clone`、`Eq`、显示或访问能力。
- `PluginName`、`Sha256Digest`、`PluginKind`、`PluginNamespace` 等可独立复用的合法值类型公开并实现 `FromStr`。
- URL 分别使用公开的 `ReleaseUrl`、`HomepageUrl` 和 `RepositoryUrl`，使各字段的 query 规则无法混用。
- 不为 `PluginManifest` 提供绕过校验的公共字段或宽松构造器。
- 本次不派生或承诺 `Serialize` 输出格式。

## 7. 错误模型

错误至少区分：

```rust
pub enum ManifestError {
    UnsupportedResolver { found: u64 },
    InvalidToml { /* parser error and span */ },
    InvalidField {
        field: ManifestField,
        reason: InvalidFieldReason,
    },
}
```

要求：

- TOML 语法和结构错误保留解析器提供的 span。
- 字段位置使用结构化的 `ManifestField`，例如 `head.repository`，而不是要求调用方解析文本。
- 字段原因使用结构化的 `InvalidFieldReason`。
- 共享校验错误不压缩为字符串，例如：

```rust
InvalidFieldReason::InvalidSlug(SlugError)
InvalidFieldReason::InvalidGitBranch(GitBranchNameError)
```

- 错误消息面向人类，但调用方依据错误 variant 做程序判断。
- 不使用 `#[non_exhaustive]` 维持假定的向后兼容。
- 同一份 manifest 存在多个错误时，确定性地返回首个错误，不聚合。

校验顺序：

1. TOML 语法、结构与字段类型。
2. `resolver` 支持情况。
3. 根字段，按照 schema 中的声明顺序。
4. `head`。
5. `dependencies`。

## 8. `ora-utils` 共享能力

当前 `main` 尚未包含 `crates/utils`。实现工作必须等待目标 `ora-utils` PR 合入，不在其他 crate 中临时复制这些规则。

PR 合入后，在 `ora-utils` 中新增：

```rust
pub use git_branch::{GitBranchName, GitBranchNameError};
pub use slug::{Slug, SlugError};
```

实现模块保持私有，并从 crate root 显式导出公共 API。不要创建容易变成杂项容器的 `validation` 公共模块。

### 8.1 Slug

`Slug` 是经过验证的拥有型值对象：

```rust
let slug = Slug::parse("ora-weather")?;
```

它保证第 5.1 节定义的单段 slug 语法和 1 至 63 字节长度限制，并返回结构化 `SlugError`。

### 8.2 Git branch name

`GitBranchName` 只建模 branch name，不提前泛化为 tag、commit 或完整 Git ref。

它静态实现 `git check-ref-format --branch` 的规则，但不启动 Git 进程，并额外拒绝 Git 的 `@{-n}` 回溯表达式。

规则至少包括：

- 接受 `feature/weather-api` 这类包含中间 `/` 的短 branch name。
- 拒绝完整的 `refs/heads/...` 路径。
- 拒绝控制字符、空格及 `~`、`^`、`:`、`?`、`*`、`[`、反斜杠。
- 拒绝 `..`、`@{` 和连续 `/`。
- 拒绝开头或结尾 `/`。
- 拒绝开头 `-`。
- 拒绝结尾 `.`。
- 拒绝任一段以 `.lock` 结尾。
- 拒绝单独的 `@`。
- 不检查 branch 是否存在于本地或远端仓库。

`ora-utils` 的英文 README 必须同步增加通用 slug 与 Git branch name 值对象的稳定职责说明。

## 9. 依赖方向

```text
ora-plugin-manifest
├── ora-utils
├── semver
├── serde
├── toml
├── url
└── thiserror
```

- `ora-utils` 保持低层、领域无关，不依赖 `ora-plugin-manifest`。
- `ora-plugin-manifest` 不依赖 `ora-plugin-manager`。
- `ora-plugin-manager` 本次也不依赖 `ora-plugin-manifest`。
- SHA-256 文本解码可直接实现为固定长度转换；除非实现证明有必要，否则不为此引入重型哈希依赖。

## 10. 测试策略

### 10.1 `ora-utils`

- slug 的合法边界、非法字符、连字符位置和长度边界。
- Git branch name 的合法嵌套路径。
- `git check-ref-format --branch` 对应的每类非法规则。
- 明确覆盖 `@{-1}`、完整 `refs/heads/...`、`.lock`、`..`、`@{` 和连续 `/`。
- 测试使用 `pretty_assertions::assert_eq` 并优先比较完整错误对象。

### 10.2 `ora-plugin-manifest`

- 完整示例成功解析，并对整个结果做深度相等比较。
- 所有可选字段缺失时成功解析。
- 空依赖表规范化为未声明依赖。
- 缺失和不支持的 resolver。
- 未知根字段、未知子表字段和未知依赖。
- 每个必填字段缺失、类型错误、空值及长度边界。
- 一个点、无点、多个点及非法 slug 段。
- namespace 和 kind 的未知值。
- SemVer 与 Ora `VersionReq` 的合法和非法输入。
- URL 的协议、凭据、query、fragment、端口和长度策略。
- SHA-256 长度、非法字符、大小写和规范化显示。
- `[head]` 的完整性及 Git branch 错误传播。
- description、license 和 branch 的首尾空白与控制字符。
- 多个错误同时存在时验证确定性的首错顺序。
- TOML 错误保留 span，字段错误保留结构化字段路径和原因。

解析测试直接传入 `&str`，不创建临时文件，因为文件系统行为不属于该 crate 的职责。

## 11. 实施顺序

1. 等待目标 `ora-utils` PR 合入，并核对合入后的 crate 边界和公开 API。
2. 在 `ora-utils` 中以测试驱动方式新增 `Slug` 和 `GitBranchName` 及其结构化错误。
3. 更新 `ora-utils` 的英文 README。
4. 新增 `crates/plugin-manifest`、workspace 配置和英文 README。
5. 先编写 manifest 解析与校验测试，再实现强类型模型和 TOML 解析。
6. 运行 changed-file formatting 和最小相关 lint/test 任务。
7. 由于会新增 workspace crate 和共享 API，最终运行仓库要求的完整 `task test`。

## 12. 完成标准

- `ora-plugin-manifest` 可以从 `&str` 解析示例 manifest。
- 所有已定义非法状态均在构造领域对象前被拒绝。
- 未知 resolver 和未知字段不会被静默接受。
- plugin-specific 规则保留在 manifest crate，通用 slug 与 Git branch 规则位于 `ora-utils`。
- 没有文件系统、网络、安装或旧格式兼容逻辑混入新 crate。
- crate README、`ora-utils` README 和相关测试与实现一致。
- 最小相关检查及完整 `task test` 均通过。
