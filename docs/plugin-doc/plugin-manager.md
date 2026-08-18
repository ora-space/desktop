## 存储路径

```
~/.ora/
└── plugins/
    ├── sources/              # 注册表源 (git 仓库/清单)
    ├── installed/            # 已安装的插件主体
    ├── data/                 # 插件运行期持久化数据
    └── cache/                # 下载的 .orax 压缩包缓存
```

## 插件分发

插件列表从中心仓拉取更新到 `~/.ora/plugins/sources/github.com/ora-space/marketplace`，类似 homebrew。插件信息放在 `registry/` 下面。

为了避免单个文件夹内文件过多，仓库内按首字母进行二级子目录划分（如 registry/a/, registry/b/）。

插件示例（`registry/o/ora-space.weather/orax.toml`）：

```toml
# 1. 基础元数据 (Metadata)
name = "user.ora-weather"
namespace = "official"
kind = "workbench"
version = "1.2.0"
description = "获取实时天气信息的 Ora 插件"
homepage = "https://github.com/user/ora-weather"
license = "MIT"

# 2. 核心发布包配置 (.orax 下载与校验)
url = "https://github.com/user/ora-weather/releases/download/v1.2.0/ora-weather-v1.2.0.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"

# 3. 开发版源码地址（可选，支持 --head 安装）
[head]
repository = "https://github.com/user/ora-weather.git"
branch = "main"

# 4. 依赖控制 (Environment & API Dependencies)
[dependencies]
ora = ">= 0.8.0"          # 限定最小支持的 Ora 版本
```

- **权限模型 (Permissions / Capabilities)：** 类似 Android 或 Deno 的 Capability 模型（如：是否允许网络访问、文件系统读写、执行 Shell 命令等）。
- 静态扫描打包后的 JS 代码，若发现使用了未在 orax.toml 中申明的危险操作（例如代码中调用了 Deno.env 但 orax.toml 未声明 env 权限），CLI 予以警告或拦截。

### orax cli

每个插件目录下有 orax.toml, logo.svg, README.md。提交 pr 到 ora-space/marketplace 跑 action，使用 orax cli 进行校验。

- `npm install -g @ora-space/orax`，实际调用 orax cli
- 用 Rust 的官方 `semver` crate 来解析和校验依赖版本
- README 校验：禁止嵌入未过滤的恶意 HTML 标签。
- SVG 校验：解析 logo.svg XML 树，禁用 `<script>` 标签、`<foreignObject>`、外部网络资源请求（如 `<image href="http://...">`），且文件限制在 50KB 以内。
- 路径安全：确保 Zip 压缩包解压路径无 ../ 路径穿越漏洞（Zip Slip 漏洞）。

- **签名与验签 (Verification & Integrity)：** 校验 `.orax` 文件的哈希（SHA-256）及数字签名（GPG / Ed25519），防止篡改或中间人攻击。

依赖与兼容性 (Dependency & Compatibility)：

- **版本兼容校验 (Version Compatibility)：** 校验宿主（Ora 版本）与插件要求的 API 版本边界（如 `semver` 匹配）。
- **依赖树拓扑求解 (Dependency Resolution)：** 插件依赖其他插件时的自动依赖下载、冲突检测（Conflict Resolution）。
- **平台适配 (Platform Matrix)：** 针对 OS（Windows / macOS / Linux）与 CPU 架构（x86_64 / aarch64）进行二进制或原生依赖的打标与拦截。

### ora-space/marketplace PR 自动化分发流水线

```
# .github/workflows/validate-plugin.yml 示例逻辑
name: Validate and Pack Plugin
on: [pull_request]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - name: Install orax cli
        run: npm install -g @ora-space/orax
      - name: Validate TOML & Assets
        run: orax validate --path registry/
      - name: Test Download & Check SHA256
        run: orax verify-release --url <release_url> --sha256 <expected_sha256>
```

### 页面展示

在 `~/.ora/plugins/cache/` 目录下生成 `registry_index.json`。

索引产物需兼顾**轻量**与**UI 展现所需的基础元数据**，避免 UI 再次逐个读取 `orax.toml`。

```json
{
  "updated_at": 1776244428,
  "version": "1.0",
  "plugins": [
    {
      "id": "official/user.ora-weather",
      "name": "user.ora-weather",
      "namespace": "official",
      "version": "1.2.0",
      "description": "获取实时天气信息的 Ora 插件"
    }
  ]
}
```

- **`id`**：由 `namespace` + `name` 生成的唯一标识。

构建触发时机放在 Sync 成功之后：

```
[git pull / sync] ──> [扫描 & 解析 registry/ 目录] ──> [生成 index.json.tmp] ──> [原子覆盖 index.json]
```

1. **并发并行解析 (Parallel Parsing)**：

- 在 Rust 中通过 `ignore::WalkBuilder` 或 `walkdir` 配合 `rayon` 多线程并发解析所有的 `orax.toml`。全量扫描数万个小 TOML 文件通常在 **50ms 以内** 完成。

2. **原子覆盖 (Atomic Swap)**：

- 构建时先写入临时文件 `registry_index.json.tmp`，解析校验全部完成后再 `std::fs::rename` 覆盖目标文件，确保 UI 在任何时刻读取到的索引均完整无损。

3. **容错机制 (Error Handling)**：

- 遇到格式错误的 `orax.toml` 时记录 warning 日志并跳过该项，避免单个坏文件阻断整个索引构建。

## 插件发现

- **本地插件发现 (Local Discovery / Scan)：** 扫描本地目录（ `~/.ora/plugins/installed`），识别已安装的插件清单、解析其 `manifest.toml`，并推导可用状态。
-

### 生命周期管理 (Lifecycle Management)

- **安装与卸载 (Installation & Uninstallation)：** 下载包、校验 Hash、解压文件，以及干净彻底地清理残留。
- **升级与回滚 (Upgrade & Rollback)：** 检查新版本、升级，以及升级失败时的原样恢复（Atomicity / Safe Rollback）。
- **状态切换 (State Control)：** 插件的**启用 (Enable)**、**禁用 (Disable)**。

后续支持：黑名单与撤销 (Revocation / Advisory)： 当某个版本的插件被发现重大安全漏洞时，中央仓推送到客户端强制禁用/下架（Vulnerability Advisory）。
