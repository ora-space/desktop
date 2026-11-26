# Marketplace

[English](marketplace.md) | 中文

Marketplace 列出发布在 Git 仓库（"源"）中的插件，负责安装和更新它们，也可以一起安装由多个插件组成的扩展包。源的 checkout 和索引位于 `~/.ora/plugins/sources/` 和 `~/.ora/plugins/cache/registry_index.json`。索引构建归 `ora-plugin-registry`；源配置、同步和安装在 `ora-backend` 中组装。包模型见[插件系统](plugin-system.zh.md)。

## 源

一个源由 HTTPS Git URL、分支（默认 `main`）、代理开关、`enabled` 标志和制品获取方式组成。操作：`listMarketplaceSources`、`addMarketplaceSource`、`updateMarketplaceSource`、`deleteMarketplaceSource`。

- **命名空间**：每个源的规范 URL（规范化大小写、凭据、默认端口、末尾斜杠和 `.git`）在 `plugin_source_namespace` 中一次性绑定到一个命名空间。这个绑定比源配置活得更久：一旦从该源安装了插件，命名空间就冻结进它的安装路径、数据目录、Skill 行和 Effect 消费者身份，所以删除再重新添加同一个仓库会复用它。两个源发布相同的 identifier 会得到两个不同的插件 id，两者都会列出。
- **启用**：禁用会保留 URL、分支、代理策略、位置和命名空间绑定，但把该源排除在同步、列表和安装之外。
- **代理**：开启「使用代理」的源跟随 [Desktop 的代理模式](../desktop/settings.zh.md#网络代理)，关闭的源始终直连。Git 和制品下载都遵守此规则。Ora 向 Git 传入本次操作的路由。明确直连的源保留 Git 的跳转行为；需要按目的地址重新选路而 Git 无法处理跳转时，请配置最终的 Git 地址。
- **制品获取**：`Direct HTTPS`（默认）获取绝对的 HTTPS 发布地址，不签名。`S3 SigV4` 接受对象 key，或在配置的 endpoint 和 bucket 上的 path-style HTTPS 地址，并用该源的 region 和静态凭据对签名；它拒绝外部的 HTTPS 地址，而不是退回到不签名的获取。endpoint、bucket、region 和两项凭据是一份完整的配置。凭据只写：源查询从不返回它们，debug 输出会脱敏。它们以明文配置的形式保存在本地 SQLite 数据库中；没有操作系统 keychain、临时凭据或 provider 凭据链。永远不要把更新凭据的请求序列化进日志或遥测。签名只认证请求，不保证完整性：每个制品仍必须匹配发布中的 SHA-256。

错误是具体的：启用 S3 却没有凭据时返回 `marketplace_s3_credentials_required`；字段无效时返回 `marketplace_artifact_retrieval_field_invalid`，带 `field` 参数（从不带它的值）。持久化的获取配置损坏属于内部错误。

### 源对话框

在 设置 → 插件 → 管理 marketplace 中，"添加源"打开一个聚焦在 URL 字段上的对话框。选择 S3 SigV4 会显示 Endpoint、Bucket、Region 和两项凭据。Backend 校验仓库身份、分支语法、S3 字段和冲突。回车提交；取消、Esc 和关闭按钮丢弃草稿但不关闭设置。提交期间阻止重复请求和关闭；失败信息留在对话框中，草稿保持不变。如果写入 S3 凭据失败，新源会被删除，而不是停留在 Direct HTTPS。之后可以通过"编辑"修改获取方式，两个凭据字段都留空表示保留已存的凭据。

## 同步与索引

`syncAvailablePlugins` 拉取每个启用的源并重建索引；`listAvailablePlugins` 读取缓存的索引；`readPluginReadme` 读取某个列表条目的 README。

- 源不存在时 clone，否则 fetch、checkout 并 fast-forward。扫描每个源的 `registry/` 目录中的 `orax.toml`；格式错误的会被跳过并记录警告。条目带有发布源的规范 URL（用来区分同名条目）、显示标题、安全时内联的 `logo.svg`，以及发布支持的目标，让 UI 在下载任何东西之前就能禁用不兼容的安装。
- 同步按源进行。失败的源保留上一份索引中它的全部条目，不重新扫描它可能写了一半的 checkout，直到之后某次同步成功。只有被删除或禁用的源才会失去条目，所以"已删除"和"不可达"永远不会看起来一样。只要原因属于某个源（它的 Git 工作或它选用的代理），失败就只归这个源；只有读取 Ora 自身状态（源、命名空间绑定）失败才会让整次同步失败。
- 失败信息存进索引，由列表和同步一起返回，所以外壳能指出哪些源的条目已过期。每条失败只带 Git 自己的 `fatal:`/`error:` 行或配置问题，从不带命令行或本地路径；完整错误写入日志。
- 只有至少一个源刷新成功，同步时间才会推进。一个源都没连上的同步保持之前的时间（或报告"从未同步"）；没有启用任何源的同步会推进时间，写下一个空目录。
- 索引文件原子替换，它是 `plugins/cache/` 中唯一持久的文件。

### 自动刷新

Desktop 通过 `ora-scheduler` 任务在启动 15 秒后刷新一次索引，之后按本地时间每六小时一次（`0 0 */6 * * *`）；开发构建也会刷新，因为它从不安装或更新任何东西。刷新失败只记录日志，直到下一次定时才重试。

Backend 同一时间只允许一次重建。重建期间到来的调用方，包括按下"同步"的用户，直接从缓存的索引得到回答，而不是排队。自动刷新期间外壳会禁用"同步"，通过原生事件 `marketplace-auto-sync-changed`（`started`、`finished`）告知；在发出 `started` 之前就已占住准入。`finished` 会让列表查询失效，所以外壳在根部挂载这个桥接。

## 安装与更新

`installPlugin` 和 `updatePlugin` 在拥有该命名空间的源上解析列表条目（因此使用它的获取方式和代理），为宿主目标选择发布，通过共享的下载管线以该源的代理和获取方式下载，边下载边校验 SHA-256，然后提交包，见[插件系统](plugin-system.zh.md#操作)。插件包可能附带数百 MB 的 Agent CLI，所以它们有自己的解压预算。

## 扩展包

`kind = "pack"` 的列表条目是一个编排入口，而不是一个包。它的 `[[pack.members]]` 写的是裸 identifier，在扩展包自己的源内解析；每个成员可以写明它适配的 Agent，这只用于展示时过滤。

- **安装**：一次静态的预检在第一个成员下载任何字节之前找出所有问题；预检失败没有任何副作用。适用的成员随后按声明顺序走普通的单插件安装链。已安装的成员被跳过，并记为 `pre_existing`。某个成员失败会停止本次运行，回滚本次创建的成员，结果中报告失败的成员以及回滚失败的成员。扩展包成员不运行 Hook `init`，因为运行成员附带的程序需要它自己的授权。
- **归属日志**：迁移 `0014` 为每个扩展包记录它创建了哪些成员（`managed_by_pack`）、找到了哪些（`pre_existing`），以及每个成员安装时的版本。每次重启后，`listPackInstallations` 把日志与已安装的目录树对账，报告每个成员是处于记录的版本、处于其他版本（用户接管了它），还是已缺失。对账只识别漂移，不修复任何东西，也从不参考当前的扩展包列表。
- **卸载**：`packUninstallPlan` 根据日志和对账算出哪些成员可以删除（由扩展包创建且仍处于记录的版本），哪些要保留以及原因（`pre_existing`、`version_changed`），哪些已经缺失。执行时逐个成员走普通卸载，并逐条释放日志记录，所以中途失败仍可重试，用户自己的插件从不被触碰。

## 决策

- [Registry 分发](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/registry/0-registry-distribution.md)
- [扩展包](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/extension-pack/0-extension-pack.md)
