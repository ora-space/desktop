# Surface

[English](surfaces.md) | 中文

Surface 是 `workbench` 或 `webview` 插件贡献的 UI，显示在隔离的原生 webview 中，可以嵌入主窗口（`embedded`），也可以是独立窗口（`windowed`）。workbench 插件随包附带一个页面，由宿主提供，并通过一个请求桥连接到插件进程；webview 插件嵌入一个外部 HTTPS 站点，没有进程。宿主内部细节见 [`apps/desktop/src-tauri/src/surface/README.md`](../../apps/desktop/src-tauri/src/surface/README.md)。

| 层 | Crate 或模块 | 负责 |
|---|---|---|
| Manifest | `ora-plugin-manifest`、`ora-plugin-manager` | `[workbench]`/`[webview]` 小节：入口页面、桥接方法、允许的 origin、下载策略 |
| 领域 | `ora-surface` | id 和 label、`SurfaceDefinition`、`NavigationPolicy`、资源 URL 与 CSP、实例状态机、`SurfaceRegistry`、下载、事件 |
| 进程 | `ora-plugin-lifecycle`，经由 `PluginGateway` | 插件数据目录、`ensure_running`、固定代的连接、`SurfaceCloser` |
| 宿主 | `apps/desktop/src-tauri/src/surface/` | Tauri webview、命令、事件、下载与宿主动作、空闲停止、`ora-plugin://` 资源、桥接 |
| 前端 | `packages/app-shell`（`SurfaceCapability`） | 占位布局、尺寸上报、下载提示和 toast、Skill 导入评审 |

宿主从不自己决定生命周期：每条命令都经过 `SurfaceRegistry`，它返回 `SurfaceEffect`，由宿主执行。注册表也是授权来源：webview label 只有在 `resolve_label` 把它映射到一条记录之后才被信任。

## 隔离

| Label | 可以调用 |
|---|---|
| `main` | 所有命令，包括 `surface_*` |
| `plugin-webview:*` | 只有 `plugin_webview_invoke` |
| `remote-webview:*` | 什么都不能；页面没有 Tauri IPC |

ACL 是外层边界；命令内部仍然会解析身份，所以即使 workbench 页面调用到了 `plugin_webview_invoke`，也只能访问它自己的实例。

surface 中的 `window.open` 从不创建 Ora webview：允许列表内的远程 URL 交给系统浏览器，所有弹窗请求都被拒绝。workbench 页面从不打开弹窗。

每个插件在数据目录下有一个持久的 web profile（`web-profile/`），所以 webview 插件能保持登录，workbench 页面即使在所有 `ora-plugin://` 页面共享同一个 origin 的平台上也有自己独立的 `localStorage`。profile 从不暴露给任何页面或 `ora/storage/*`。

## 打开与关闭

`surface_open` 从已安装的 manifest 解析 surface，注册一个 `Opening` 实例，创建 webview，在注册表中以 `Opened` 完成，并发出 `opened`。打开不会启动插件进程：workbench 页面的第一次桥接调用才会启动它。webview surface 是单例（再次打开会聚焦已有窗口）；workbench surface 可以有多个实例。

独立窗口的实例把 `CloseRequested` 转为注册表中的关闭；关闭从不被阻止。销毁主窗口会关闭所有 surface。生命周期在停止或卸载插件之前调用 `SurfaceCloser`，所以它的 surface 会先关闭。插件的最后一个实例关闭后，启动一个 30 秒的空闲计时器，如果期间没有重新打开实例，就停止进程。

## 命令与事件

| 命令 | 用途 |
|---|---|
| `surface_capabilities` | `{ embedded }` |
| `surface_list` | 当前的 `SurfaceRecord` |
| `surface_open` | 打开 `{ pluginId, target }`；不支持时 `embedded` 降级为 `windowed` |
| `surface_close`、`surface_reload` | 关闭；重新加载页面（`failed` 的实例会重建） |
| `surface_set_bounds`、`surface_set_visible` | 以 CSS px（Tauri 逻辑单位）放置嵌入的实例 |
| `surface_popout`、`surface_dock` | 在独立窗口和嵌入之间切换 |
| `surface_resolve_download`、`surface_discard_download` | 答复一个需要提示的下载 |

`SurfaceRecord` 是 `{ instance, pluginId, kind, title, target, state }`，`state` 取值为 `opening`、`open`、`migrating`、`closing` 或 `failed`。错误使用共享契约：`plugin_not_found`、`resource_in_use`、`invalid_request`、`internal_error`。`surface://event` 携带 `SurfaceEvent`（`opened`、`migrated`、`migrateFailed`、`failed`、`closed`、`downloadStarted`、`downloadChoice`、`downloadCompleted`、`downloadFailed`）；它在 `packages/app-shell/src/platform/types.ts` 中的 TypeScript 对应定义必须与 Rust 的 serde 形状一致。

`embedded-surfaces` Cargo feature（`tauri/unstable`，默认关闭）编译子 webview 和重新挂接父窗口的能力。`surface_capabilities.embedded` 是这个编译开关与运行时探测的组合（没有设置 `GDK_BACKEND=x11` 的 Linux Wayland 会话报告 `false`）。

## Workbench surface

workbench 插件声明入口页面和页面可以调用的桥接方法；打开实例时两者都冻结进 `SurfaceDefinition`。

**资源**：页面加载 `ora-plugin://localhost/<instance>/<entry>`（Windows 上是 `http://ora-plugin.localhost/...`）。协议处理器通过注册表解析调用方的 label（以 label 而不是 URL 为准），要求 URL 中的实例片段一致，路径是资源根目录内的 `PortableRelativePath`，是普通文件且内容类型已知。所有拒绝都是不带信息的 404，原因写入日志。文档以 `Cache-Control: no-store` 返回，其他资源用 `no-cache`；文档带有禁止内联脚本和样式、远程内容和 frame 的 CSP。

**桥接**：宿主注入 `window.ora.invoke(method, params)`，它调用 `plugin_webview_invoke`。宿主把调用方解析为一个活着的实例，按 manifest 的允许列表检查方法，请求和响应各限 1 MiB，必要时启动插件（15 秒），把允许列表与运行中那一代注册的方法取交集，再用宿主拥有的信封调用方法：

```jsonc
{ "surface": { "instance_id": 7, "generation": 3 }, "input": /* 页面参数 */ }
```

页面无法指定插件、实例或代。拒绝要么是 `{ kind: "host", code }`（`SURFACE_UNAVAILABLE`、`PAYLOAD_TOO_LARGE`、`METHOD_NOT_ALLOWED`、`METHOD_UNAVAILABLE`、`PLUGIN_UNAVAILABLE`、`PLUGIN_CALL_TIMED_OUT`、`INTERNAL`），要么是 `{ kind: "plugin", code, message }`，插件的消息会去掉控制字符并限制在 1 KiB，所以插件无法冒充宿主的情况。

**代绑定**：实例固定在它的桥接第一次到达的插件代上。之后的调用如果看到另一代（进程崩溃或重启了），会关闭这个过期实例并以 `SURFACE_UNAVAILABLE` 失败；重新打开会得到新实例。由于生命周期不发出退出通知，这一步发生在实例的下一次调用时。

## Webview surface

webview 插件写明起始 URL 和一组精确的允许 HTTPS origin；导航到它们之外会被拒绝并记录。

### 下载

manifest 把页面 URL 规则映射到处理方式：`{ reject }`、`{ auto = "import_skill" }` 或 `{ prompt = [actions...] }`，动作为 `import_skill` 和 `save_as`（`save_as` 需要目标位置，所以不能用于 `auto`）。下载只按 webview label 归属，处理方式按发起页面的 URL 选择，文件先以唯一的 `.part` 落在 `data/<namespace>/<name>/webview/downloads/` 中，成功时改为正式名称，失败时删除。

- **提示**：落地的文件停在 `AwaitingChoice`，`downloadChoice` 发给主 webview，后者一次显示一个排队的提示，用 `surface_resolve_download`（运行选中的动作）或 `surface_discard_download`（删除文件）答复。一个下载永远不会运行两个动作。
- **自动**：宿主自己运行动作。只有动作成功之后才发出 `downloadCompleted`；对 `import_skill` 它带有 `importSessionId`，前端据此打开 Skill 导入评审，见 [Agent 定义与 Skill](../agents/agents-and-skills.zh.md#导入-skill)。动作失败会发出 `downloadFailed` 并删除文件。
