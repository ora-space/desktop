# 插件 UI:资产投递与安全模型

本文覆盖**插件前端页面(Panel)的静态资产如何进入 webview**,以及围绕它的隔离与权限设计。

不覆盖:插件后端进程的沙箱方案(见 [plugin-deno.md](plugin-deno.md))、JSON-RPC 帧协议与生命周期监督(见 [plugin-runtime.md](plugin-runtime.md))、分发与安装(见 [plugin-manager.md](plugin-manager.md))。

文中标注 ✅ 的是本轮已定的决策,🔶 是待决事项,🔬 是需实机验证的假设。

---

## 1. 现状盘点

### 已有且可复用

| 资产                | 位置                                                                                    | 与本设计的关系                                                                              |
| ------------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| 插件发现            | [`ora-plugin-manager`](../../crates/plugin-manager)                                     | 产出 `InstalledPlugin` 不可变快照,是资产服务的唯一数据来源                                  |
| 路径包含性校验      | [`ora-fs`](../../crates/fs/src/path.rs) 的 `PortableRelativePath` / `CanonicalPathRoot` | 协议 handler 直接复用,不另写路径安全逻辑                                                    |
| 校验调用范式        | [`validate_main_path`](../../crates/plugin-manager/src/validation.rs)                   | 资产根、入口文件的校验照此结构                                                              |
| 不受信 webview 范式 | [`skill_marketplace.rs`](../../apps/desktop/src-tauri/src/skill_marketplace.rs)         | 独立 label、独立 `data_directory`、`on_navigation` 白名单、`on_new_window` 拒绝——全部可复用 |
| 权限闸门            | [`capabilities/default.json`](../../apps/desktop/src-tauri/capabilities/default.json)   | `"windows": ["main"]` 已使非 `main` 窗口默认零 Tauri 权限                                   |

**关于权限闸门**:Tauri v2 的 capability 按 window/webview label 授权。Ora 目前把包括 `core:default` 在内的全部权限**只**授给 label 为 `main` 的窗口,marketplace 的两个窗口正是靠这个默认值在零权限下运行。因此"插件面板不能乱调后端命令"**不是要新建的机制,而是只要不破坏现有默认值就成立的性质**。

### 尚不存在

- 面板资产的协议 handler(仓库内无任何 `register_uri_scheme_protocol` 调用)
- manifest 中的面板声明字段([`manifest.rs`](../../crates/plugin-manager/src/manifest.rs) 只有 `ora.main` 与 `ora.contributes.agents`)
- 面板相关的 contracts DTO([`contracts/plugin.rs`](../../crates/contracts/src/plugin.rs) 无 panel/logo 字段)
- 面向面板 webview 的独立 capability 文件

### 一处需要注意的既有配置

[`tauri.conf.json`](../../apps/desktop/src-tauri/tauri.conf.json) 中 `"csp": null`,主窗口无 CSP 限制。面板**不得沿用**此配置,详见第 6 节。

---

## 2. 资产传输方式选型

✅ **决策:自定义 URI scheme,使用 `register_asynchronous_uri_scheme_protocol`。**

| 方案                                                     | 判断                                                                                                                                       |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| **自定义 URI scheme**                                    | ✅ 真实 URL,相对路径 import / `<link>` / 动态 `import()` 正常工作;可逐请求设置响应头(Content-Type、CSP、nonce);异步版本不阻塞 webview 线程 |
| Tauri 内置 `asset:` 协议                                 | ❌ 仅 glob scope 白名单,无自定义响应头;需开启 `assetProtocol` capability,授权面大于所需                                                    |
| HTML 字符串直接注入(data: URL / `initialization_script`) | ❌ 相对资源引用全断,CSS/JS 无法拆分,体积受限                                                                                               |
| 本地 HTTP server                                         | ❌ 端口管理、需自建 token 鉴权、Windows 防火墙弹窗;凭空增加一个任何本机进程都可访问的攻击面                                                |

---

## 3. URL 形状与平台差异

以下形状**已从 [tauri 2.11.5](../../apps/desktop/src-tauri/Cargo.toml) 源码确认**(`src/app.rs:2126-2127` 的文档注释):

```
macOS / iOS / Linux:   ora-plugin://localhost/<plugin_id>/<path>
Windows / Android:     http://ora-plugin.localhost/<plugin_id>/<path>
```

### 关键约束:插件 id 只能放在 path,不能放在 host

Windows 下 host 段被 `<scheme>.localhost` 占满,无法承载插件 id;而 scheme 必须在 `tauri::Builder` 构建期注册,插件是运行时才发现的,"一个插件一个 scheme"不可行。

**因此所有插件面板共享同一个 origin。** 这否定了依靠浏览器同源策略做插件间隔离的思路——替代方案见第 5 节。

---

## 4. iframe / webview 抉择

✅ **决策:方案 B,面板加载进独立的 webview,不做主窗口内的 iframe。**

|                        | A. 主窗口内 iframe                                                                       | **B. 独立 webview**                              | C. 主窗口内普通 iframe(无 sandbox)             |
| ---------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------ | ---------------------------------------------- |
| UX                     | 停靠式面板,与现有 React 结构自然融合                                                     | 独立窗口;或用 multiwebview 取得停靠视觉          | 同 A                                           |
| 消息路径               | 面板 → `postMessage` → **主窗口 JS** → Tauri 命令 → Rust                                 | 面板 → Tauri 命令 → Rust                         | 同 A                                           |
| **插件身份绑定在哪**   | **TypeScript**(主窗口靠 `event.source` 比对 iframe),Rust 只能相信主窗口传来的 `pluginId` | **Rust**(webview label → plugin_id),前端无法伪造 | 同 A                                           |
| 与最高权限上下文的接触 | 主窗口(拥有全部 capability)直接处理插件消息                                              | 无接触                                           | 无接触,但共享 origin 下插件间可互读 DOM 与存储 |
| 结论                   | 可行但有降级                                                                             | ✅ 采用                                          | ❌ 排除                                        |

### 选 B 的理由

整套模型里所有安全判定——路径包含、capability 鉴权、CSP 下发——都在 Rust 手里。方案 A 会把**身份判定**这一项单独下沉到 TypeScript,成为唯一一处贴近不受信侧的安全判定。VS Code 走 A 的形状是被 Electron 架构所迫,并非因其更优;Tauri 给了不必如此的选择。

若采用 A,则主窗口中那段 iframe → 命令的中继必须按安全边界代码来写和测,不能当作普通 UI 胶水。

🔶 **待决:面板形态取独立窗口,还是 Tauri multiwebview(需开启 `unstable` feature)以获得停靠视觉。** 两者的 Rust 侧代码完全一致,切换只影响窗口构建部分,可先用独立窗口跑通模型。

---

## 5. 共享 origin 问题的解法

方案 B 下没有父文档可挂 iframe,`sandbox` 属性造 opaque origin 的路子用不上。但 B 恰好提供了更强的替代。

### 核心机制:宿主侧归属校验

**已从 tauri 2.11.5 源码确认**(`src/app.rs:2469-2483`):

```rust
pub struct UriSchemeContext<'a, R: Runtime> {
    pub(crate) app_handle: &'a AppHandle<R>,
    pub(crate) webview_label: &'a str,
}

/// Get the webview label that made the uri scheme request.
pub fn webview_label(&self) -> &'a str
```

协议 handler 对**每一个**资源请求都知道发起方是哪个 webview,因此可以在宿主侧强制归属:

```rust
let plugin_id = panel_registry.plugin_for_webview(ctx.webview_label())?;  // label → plugin_id
let requested   = path.first_segment()?;                                  // URL 声明的归属
if requested != plugin_id {
    return not_found();   // 插件 A 的 webview 取不到插件 B 的任何字节
}
```

✅ **决策:此规则无任何例外分支。** 请求方 label 必须是已注册的面板 webview,且其 plugin_id 必须等于 URL 首段,否则 404。无例外意味着该规则可被表驱动测试完全覆盖,也不会在迭代中被逐渐添加的特例侵蚀。

### 为什么这比 origin 隔离更强

|          | origin 隔离(VS Code) | label 归属校验(本方案)      |
| -------- | -------------------- | --------------------------- |
| 判定方   | 浏览器引擎           | Rust 宿主                   |
| 判定依据 | URL 长什么样         | **谁在请求**                |
| 粒度     | 到 origin 为止       | 精确到插件,可进一步到单文件 |
| 可测试性 | 依赖 webview 行为    | 纯函数,可单测               |

VS Code 必须造 `vscode-webview://<uuid>` 这个唯一 authority,是因为其资源加载判定最终落在浏览器侧,**只能用 URL 编码身份**。方案 B 下 Ora 直接持有请求方身份,无需再用 origin 作为身份的代理变量。

由此,`ora-plugin://` 的 origin 退化为纯命名空间,不承担安全语义。安全判定收敛于 Rust 一处,而非分散在 URL 形状、CSP、浏览器同源策略三处。

### 三处残留风险

归属校验只作用于经过 handler 的请求,以下三条需单独关闭。

**残留 1 — 共享 origin 下的存储互通**(localStorage / sessionStorage / IndexedDB / BroadcastChannel / SharedWorker / cookie)

两个手段,推荐以后者为主力:

- 每个面板 webview 独立 `data_directory`。API 在 `WebviewBuilder`(`src/webview/mod.rs:963`)与 `WebviewWindowBuilder`(`src/webview/webview_window.rs:1024`)上均存在,marketplace 已在使用。
- **不给面板任何 Web 存储**:`initialization_script` 在插件脚本执行前移除 `localStorage` / `sessionStorage` / `indexedDB` / `BroadcastChannel` / `SharedWorker`,状态一律经 `getState` / `setState` 桥由宿主持有。此做法与 VS Code 的设计意图一致——VS Code 之所以提供 `setState`/`getState`,正是因为 webview 不应依赖自身存储。

  ⚠️ **配套约束**:插件可通过 `document.createElement('iframe')` 创建同源 iframe,从 `iframe.contentWindow.localStorage` 取回干净的全局对象。因此必须同时下发 CSP `default-src 'none'`(`frame-src` 随之默认拒绝)。**两者缺一则整条不成立。**

🔶 **待决:存储策略取 `data_directory` 隔离,还是完全不给 Web 存储。** 推荐后者;若采纳后者,则下面的验证项不再影响可行性。

🔬 **待验证:multiwebview 模式下,同一窗口内的多个子 webview 是否真的各自获得独立的 WebView2 environment。** 这是 wry 的实现细节,源码层面无法判定。建议 spike 与实现并行,不要阻塞。

**残留 2 — CSP 中 `'self'` 的作用域过宽**

有了真实 origin 后,`'self'` 等于整个 `ora-plugin://localhost`,覆盖所有插件的路径——`script-src 'self'` 将允许插件 A 加载插件 B 的脚本。

故不使用 `'self'`。CSP source 表达式支持路径前缀,`cspSource` 应生成为:

```
ora-plugin://localhost/<plugin_id>/          # macOS / Linux
http://ora-plugin.localhost/<plugin_id>/     # Windows
```

这是 VS Code `webview.cspSource` 在 Ora 中的具体取值。此项为纵深防御——归属校验已在宿主侧拦截,CSP 能拦的它都能拦。

**残留 3 — 跨 webview 获取窗口句柄**

同源仅在持有对方 window 引用时才有杀伤力。Rust 独立创建的 webview 之间无 `opener`、无 `frames` 关系,唯一途径是 `window.open('', 'named')` 命名窗口定位。`on_new_window` → `Deny`(marketplace 现成写法)即可关闭。

---

## 6. CSP 基线

面板**不得**沿用主窗口的 `"csp": null`。因走自定义协议 handler,CSP 可逐插件、逐响应下发,无需改动全局配置。

```
default-src     'none';
script-src      'nonce-<每次响应生成>';
style-src       <cspSource>;
img-src         <cspSource> data:;
connect-src     'none';
frame-ancestors 'none';
```

- `default-src 'none'` 是残留风险 1 的配套项,不可省略。
- `connect-src 'none'` 是整个权限模型的承重墙:面板需访问网络时必须经 JSON-RPC 桥转交插件后端进程,再由 Rust 按声明的 capability 二次鉴权。否则插件把数据放到前端一个 `fetch` 即可绕过 [plugin-runtime.md](plugin-runtime.md) 中 Capability Guard 的全部约束。
- nonce 方案同时解决了"是否开放 `style-src 'unsafe-inline'`"的取舍:handler 每次响应生成 nonce 注入 HTML,插件按约定书写 CSP,既不开放 `unsafe-inline`,插件又能写内联脚本。

🔶 **待决:`style-src` 是否给 `'unsafe-inline'`。** 不给则使用 Tailwind / CSS-in-JS 的插件基本无法工作;给则等于接受样式注入。是 DX 与严格度的直接取舍。

---

## 7. Handler 处理链

```
请求 ora-plugin://localhost/<plugin_id>/assets/app.js
  │
  1. ctx.webview_label() → 查面板注册表 → plugin_id
  │    ↳ 非已注册面板 webview → 404（无例外，main 窗口同样拒绝）
  │
  2. URL 首段 plugin_id 与步骤 1 结果比对
  │    ↳ 不匹配 → 404
  │
  3. 取该插件面板资产根（manifest 声明，非 package_root）
  │    CanonicalPathRoot::new(panel_root)
  │
  4. PortableRelativePath::parse(剩余 path)
  │    ↳ 拒绝 ..、盘符前缀、UNC、NUL —— crates/fs/src/path.rs 已实现
  │
  5. root.resolve_existing(&relative)
  │    ↳ canonicalize 后复核 starts_with(root)，符号链接指向包外即拒
  │
  6. 扩展名 → Content-Type（固定白名单，不做内容嗅探）
  │    ↳ 白名单外的扩展名直接 404，不回落 octet-stream
  │
  7. 附加响应头返回字节
       Content-Security-Policy: <按该插件生成，含本次 nonce>
       X-Content-Type-Options: nosniff
```

步骤 3–5 与 [`validate_main_path`](../../crates/plugin-manager/src/validation.rs) 校验 `ora.main` 的调用序列一致,符合 AGENTS.md 中"路径校验优先复用 `ora-fs`"的要求,并避免两套路径安全逻辑口径漂移。

### 已知残留:TOCTOU

[plugin-manager README](../../crates/plugin-manager/README.md) 已载明 canonicalize 无法阻止校验与实际打开之间的符号链接替换。资产协议中该窗口远小于 discovery(每请求现算),若要彻底关闭,应在 `File::open` 之后用 fd 的 metadata 复核是否为常规文件,而非依赖 open 之前的路径判断。

---

## 8. 插件 logo 通道

✅ **决策:logo 不走 `ora-plugin` 协议。** 这使第 5 节的归属校验保持无例外。

推荐**专用 unary 命令 + 懒加载**,而非塞入 `list_installed_plugins`:

```rust
get_plugin_logo(pluginId) -> Option<String>   // data: URI
```

理由:[`ListInstalledPluginsResponse`](../../crates/contracts/src/plugin.rs) 是插件列表每次渲染都要调用的高频接口,每插件挂载 50 KB base64(logo.svg 上限 50 KB,base64 后再 +33%)会显著加重它;[desktop-runtime.md](../desktop-runtime.md) 中已有"文件字节不经 Tauri IPC"的既定倾向。`main` 窗口本就持有全部 capability,新增命令无需改动 capability 配置。

### ⚠️ 真正的风险在渲染,不在传输

**SVG 是可执行内容,而 `main` 窗口是全应用权限最高的上下文。**

| 渲染方式                                       | 结果                                                  |
| ---------------------------------------------- | ----------------------------------------------------- |
| `<img src="data:image/svg+xml;base64,...">`    | ✅ 浏览器以非脚本化模式渲染:脚本不执行,外部资源不加载 |
| CSS `background-image: url(data:...)`          | ✅ 同上                                               |
| 内联 `<svg>` 进 DOM(`dangerouslySetInnerHTML`) | ❌ **插件代码在主窗口 origin 中执行**                 |

第三行需说明精确:`innerHTML` 不执行 `<script>` 元素,但 SVG 元素上的 `onload` / `onbegin`(`<animate>`)等事件属性**会**触发,故 `<svg onload="...">` 可被利用。

**硬性规则:插件 logo 只能经 `<img>` 或 CSS background 渲染,永不内联进 DOM。** 建议以前端 lint 约束钉死——仓库已有[禁止渲染 `Error.message` 的 restricted-syntax 规则](../frontend-contract-sdk.md)作为同类先例。

一旦走 `<img>`,SVG 内是否含 `<script>` 便不再是承重问题,宿主侧的 SVG 净化降级为纵深防御。

### 生产者侧校验不可信

[plugin-manager.md](plugin-manager.md) 中的 SVG 校验(禁 `<script>` / `<foreignObject>` / 外部引用、≤50 KB)由 orax cli 在 marketplace CI 中执行,属**生产者侧**。插件可从本地目录、`--head` 源码或手工放入 `<data_dir>/plugins/` 安装,完全绕过 CI,宿主不能将其视为已成立的前提。

配合 `<img>` 渲染后,宿主侧仍需自行兜底的仅剩两条:

- **字节上限**:bounded read,复用 [`read_bounded`](../../crates/plugin-manager/src/discovery.rs) 中"多读一字节以探测超限"的写法
- **拒绝 XML DTD / 实体声明**:`<img>` 挡得住脚本,挡不住 billion laughs 类实体展开炸弹

---

## 9. VS Code 对照

### 可近乎原样借鉴

| VS Code 机制                                                            | 映射到 Ora                                                        |
| ----------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `localResourceRoots`                                                    | 协议 handler 的资产根边界,以 `CanonicalPathRoot` 实现             |
| `webview.asWebviewUri(fileUri)`                                         | SDK 上的路径→URL 转换函数;转换时即做根包含性校验,越界路径转不出来 |
| `webview.cspSource`                                                     | **必需项**,取值见第 5 节残留 2                                    |
| `acquireVsCodeApi()`(仅 `postMessage`/`getState`/`setState`,只能调一次) | 窄口径桥的口径参照;VS Code 验证了该口径足够小也足够用             |
| nonce + `script-src 'nonce-...'`                                        | 见第 6 节                                                         |
| `getState` / `setState`                                                 | 面板状态由宿主持有,与"不给 Web 存储"策略互为因果                  |
| `enableScripts` 默认 `false`                                            | 纯展示型面板无需 JS,默认关闭可削减大片攻击面                      |

### 不应借鉴

**VS Code 的 extension host 没有沙箱**——扩展运行在 Node.js 中,拥有完整的文件系统、网络与子进程权限。其立场是"扩展受信,webview 才是需防护的 UI 表面"。

而 [plugin-deno.md](plugin-deno.md) 收敛的结论相反:插件进程本身即需锁死(Deno `--no-prompt`,子进程由宿主 broker 代管)。**Ora 在后端侧比 VS Code 严格得多,这是正确的,不应为"像 VS Code"而放松。**

结论:**借鉴 webview 层,不借鉴 extension host 层。** 这两层在 VS Code 中是解耦的,可分别取舍。

---

## 10. 安全模型总表

| 威胁                          | 缓解手段                                                                     | 强制点                      |
| ----------------------------- | ---------------------------------------------------------------------------- | --------------------------- |
| 路径穿越读取宿主文件          | `PortableRelativePath` + `CanonicalPathRoot::resolve_existing`               | Rust handler                |
| 包内符号链接指向包外          | 同上(canonicalize 后复核包含性)                                              | Rust handler                |
| 插件 A 读取插件 B 的资产      | **label → plugin_id 归属校验**                                               | Rust handler                |
| 插件 A 读取插件 B 的存储      | 不给 Web 存储 + `default-src 'none'`;或独立 `data_directory`                 | initialization_script + CSP |
| 面板调用主应用的 Tauri 命令   | capability 按 label 授权,面板 label 不入 `default.json`                      | Tauri ACL                   |
| 面板直接发起网络请求          | `connect-src 'none'`                                                         | CSP                         |
| 面板导航至远程页面 / 开新窗口 | `on_navigation` 白名单 + `on_new_window` → `Deny`                            | Rust 窗口构建               |
| MIME 混淆(.txt 当脚本执行)    | 固定扩展名白名单 + `X-Content-Type-Options: nosniff`                         | Rust handler                |
| logo SVG 在主窗口执行脚本     | 只经 `<img>` / CSS background 渲染                                           | 前端 lint 约束              |
| logo 实体展开炸弹             | bounded read + 拒绝 XML DTD                                                  | Rust discovery              |
| 安装期资源耗尽                | 参照 [`skill-package` 的 `Limits`](../../crates/skill-package/src/limits.rs) | 安装流程                    |

---

## 11. 需新增与改动的清单

| 位置                                                             | 内容                                                                                                                   |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| [`manifest.rs`](../../crates/plugin-manager/src/manifest.rs)     | `ora.contributes.panels[]`:`id` / `title` / `root`(资产目录)/ `entry`(入口 HTML);`ora.logo`(相对路径)                  |
| [`validation.rs`](../../crates/plugin-manager/src/validation.rs) | 仿 `validate_main_path` 校验 `root`、`entry`、`logo` 的包内包含性,并保证 `entry` 落在 `root` 之内                      |
| [`contracts/plugin.rs`](../../crates/contracts/src/plugin.rs)    | `InstalledPlugin` 增加 `panels` 字段;新增 `get_plugin_logo` 的请求/响应 DTO                                            |
| `capabilities/plugin-panel.json`                                 | 新 capability 文件,`windows` 匹配面板 label,`permissions` **仅含**插件桥命令                                           |
| 新协议 handler 模块                                              | 挂载于 `tauri::Builder`,持有已存在于 [`DesktopState`](../../apps/desktop/src-tauri/src/lib.rs) 的 `Arc<PluginManager>` |
| 前端 lint 规则                                                   | 禁止内联渲染插件 SVG                                                                                                   |

### 一处需与 `ora.main` 不同的失败语义

当前 [`validate()`](../../crates/plugin-manager/src/validation.rs) 中任一项失败即返回 `Err`,整个插件被 discovery 跳过。`main` 解析失败理应跳过——插件根本无法运行。但 **logo 损坏不应使插件消失**,它只是个图标。

故 logo 应为 `Option` 语义:校验失败时记录一条 `PluginDiscoveryIssue`(与现有跳过整包时所记同构),插件本身照常进入快照,UI 回落至默认图标。这需要在 `InstalledPlugin` 上区分"致命字段"与"降级字段",是本次改动中唯一触及现有校验结构的地方。

---

## 12. 两个解耦性质

**资产服务不依赖插件后端进程。** 它只依赖 `PluginManager` 快照,因此面板可先渲染,JSON-RPC 桥连接失败再降级提示。两条链路可独立开发与测试。

**但快照会过期。** `PluginManager` 是[启动时的不可变快照,且明确不监听文件系统](../../crates/plugin-manager/README.md)。运行时安装/卸载插件后,handler 会持过期快照继续服务已卸载插件的文件。实现安装功能时,此处需换为可刷新的共享状态。

---

## 13. 待决与待验证汇总

### 🔶 待决

| 项                                                    | 影响                                                                                |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------- |
| 面板形态:独立窗口 vs multiwebview(`unstable` feature) | 仅影响窗口构建代码与 UX,Rust 侧其余部分不变                                         |
| 存储策略:`data_directory` 隔离 vs 完全不给 Web 存储   | 推荐后者;采纳后者则下方验证项不再阻塞                                               |
| `style-src` 是否给 `'unsafe-inline'`                  | DX 与严格度的直接取舍                                                               |
| 开发模式是否允许面板指向 dev server                   | 若允许,**必须硬性限定仅 debug 构建生效**;否则"允许远程 origin 进面板"将击穿整套模型 |

### 🔬 待验证

| 项                                                     | 说明                                                |
| ------------------------------------------------------ | --------------------------------------------------- |
| multiwebview 下子 webview 的 `data_directory` 是否独立 | wry 实现细节,源码层面不可判定;建议 spike 与实现并行 |

### 上游未决(影响有限,记录备查)

插件运行时引擎 Deno vs Bun 在 [plugin-deno.md](plugin-deno.md) 中尚无最终结论,而 [`manifest.rs`](../../crates/plugin-manager/src/manifest.rs) 已将字段固定为 `engines.bun`。此项不影响本文的资产投递设计,但若最终选定 Deno,该字段名需一并调整。
