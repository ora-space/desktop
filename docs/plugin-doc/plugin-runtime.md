## 插件开发与打包

使用 `deno create jsr:@ora-space/create-plugin my-weather-plugin` 快速创建插件项目。

deno.json

```json
{
  "name": "@user/ora-weather",
  "version": "1.2.0",
  "exports": "./main.ts",
  "tasks": {
    "dev": "deno run --watch main.ts",
    "build": "deno run -A scripts/build.ts",
    "pack": "deno task build && orax pack",
    "check": "deno check main.ts"
  },
  "imports": {
    "@ora/sdk": "jsr:@ora-space/sdk@^1.0.0"
  },
  "compilerOptions": {
    "strict": true,
    "lib": ["deno.ns", "dom"]
  },
  "fmt": {
    "useTabs": false,
    "lineWidth": 80,
    "indentWidth": 2
  },
  "lint": {
    "rules": {
      "tags": ["recommended"]
    }
  }
}
```

- 日常开发/调试：`deno task dev`
- 本地构建校验：`deno task build`
- 打包 .orax 准备发布：`deno task pack`（自动触发 scripts/build.ts，然后由 orax cli 进行 SVG/TOML 校验并压缩）

插件压缩包下载后直接解压至 ~/.ora/plugins/installed/<plugin_id>/ 目录。

```
ora-weather-v1.2.0.orax
├── orax.toml          # [必须] 插件元数据与 Capability 声明
├── main.js            # [必须] 插件入口文件
├── logo.svg           # [必须] 图标（需过 orax cli 的安全与尺寸校验）
├── README.md          # [必须] 插件说明文档
└── assets/            # [可选] 静态资源目录（图片、配置模板等）
```

### 插件签名

## 进程与隔离模型 (Process & Isolation Model)

使用 `deno run --no-prompt main.ts` 运行插件进程

### Capability 映射与启动 Flag 构建

`orax.toml` 中声明的 Capability，必须在 Ora 拉起 Deno 子进程时**精确翻译**为 Deno CLI Flags：

| `orax.toml` Capability          | 映射到 Deno 启动参数                            | 限制/安全原则                                              |
| ------------------------------- | ----------------------------------------------- | ---------------------------------------------------------- |
| `network = ["api.weather.com"]` | `--allow-net=api.weather.com`                   | 限制具体的域名/IP，禁止盲目开放全网权限                    |
| `fs.read = ["data"]`            | `--allow-read=~/.ora/plugins/data/<plugin_id>`  | 绝对禁止读取宿主敏感目录（如 `~/.ssh` 或 `~/.ora/config`） |
| `fs.write = ["data"]`           | `--allow-write=~/.ora/plugins/data/<plugin_id>` | 严格隔离在插件自身的 `data/` 目录                          |
| `env = ["LOG_LEVEL"]`           | `--allow-env=LOG_LEVEL`                         | 仅读取指定的环境变量                                       |
| `shell = true`                  | `--allow-run`                                   | **特权权限**，默认严格禁止，仅限高信任度/官方插件          |

**示例命令构建：**

```bash
deno run \
  --no-prompt \
  --allow-read="/Users/user/.ora/plugins/data/official.user.ora-weather" \
  --allow-write="/Users/user/.ora/plugins/data/official.user.ora-weather" \
  --allow-net="api.weather.com" \
  /Users/user/.ora/plugins/installed/official.user.ora-weather/main.ts
```

### 资源限制与安全兜底 (Resource Constraints)

仅依靠 Deno 权限还不够，需要对子进程进行资源约束：

1. **内存上限 (V8 Max Old Space Size)**：

- 通过 `--v8-flags=--max-old-space-size=256` 限制单个插件内存上限（如 256MB），超过直接被 V8 OOM Killer 挂掉，防止内存泄漏挤爆宿主。

2. **崩溃与重启策略 (Watchdog & Circuit Breaker)**：

- **崩溃重试**：子进程异常退出（Exit Code != 0），Ora 进行指数退避重启（Exponential Backoff）。
- **熔断机制**：1 分钟内崩溃超过 3 次，将插件标记为 `Failing` 状态，停止自动重启，并在 UI 提示用户“插件发生故障，已自动禁用”。

3. **优雅退出 (Graceful Shutdown)**：

- Ora 退出或禁用插件时，通过 Stdio 发送 `shutdown` RPC 消息，等待插件完成清理（设定 Timeout，如 3s）。
- 超时后依次发送 `SIGTERM` -> `SIGKILL` 强行终止子进程。

## IPC 通信与 SDK 注入

插件使用大端序通过 stdio 通信，数据格式：`[length i32][type i8][payload]`。Rust 侧用 `tokio_util::codec::LengthDelimitedCodec` 编解码。注意零拷贝。

```rs
#[repr(i8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    /// Standard JSON-RPC 2.0 Message
    JsonRpc = 0x01,
}

#[derive(Debug, Clone)]
pub struct RawFrame {
    pub frame_type: FrameType,
    pub payload: Bytes,
}

/// 业务层消息抽象
#[derive(Debug, Clone)]
pub enum Message {
    /// 0x01: JSON-RPC 消息
    JsonRpc(JsonValue),
    /// 未知或未处理的 Frame（用于向下兼容）
    Raw {
        raw_type: i8,
        payload: Bytes,
    },
}

pub struct IpcCodec {
    inner: LengthDelimitedCodec,
}

impl IpcCodec {
    pub fn new() -> Self {
        let inner = LengthDelimitedCodec::builder()
            .big_endian()
            .length_field_length(4)
            .max_frame_length(16 * 1024 * 1024)
            .new_codec();

        Self { inner }
    }
}
```

### 1. 二进制帧协议定义 (Framing Protocol)

全量数据包在管道传输时的字节布局如下：

```text
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Length (i32 BE)                         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   Type (i8)   |               Payload (JSON / Bytes) ...      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

```

- **Length (`i32`, 4 Bytes, Big-Endian)**：数值表示后续数据包的字节长度，即 `1 (Type) + N (Payload)`。
- **Type (`i8`, 1 Byte)**：消息枚举类型，用于区分通信意图。
  - `0x01`：jsonrpc
  - 其余待定
- **Payload (`N Bytes`)**：如果 type 为 0x01，则为 UTF-8 编码的 **JSON 字符串**。

### 2. Deno 端与 SDK 依赖注入 (`@ora-space/sdk`)

Deno 进程启动后，全靠 `Deno.stdin` 与 `Deno.stdout` 进行流式读写。SDK 内部会封装一套轻量级的 Frame 流解析器。

#### (1) SDK 内部拆包逻辑 (DataView & Stream Parser)

```typescript
// sdk/src/transport.ts
export class OraTransport {
  private pendingRequests = new Map<string, (res: any) => void>();
  private requestId = 0;

  constructor() {
    this.listenStdin();
  }

  // 1. 读取 Deno.stdin 字节流并进行粘包拆包
  private async listenStdin() {
    const reader = Deno.stdin.readable.getReader();
    let buffer = new Uint8Array(0);

    while (true) {
      const { value, done } = await reader.read();
      if (done) break;

      // 追加新读取的 chunk
      const nextBuf = new Uint8Array(buffer.length + value.length);
      nextBuf.set(buffer);
      nextBuf.set(value, buffer.length);
      buffer = nextBuf;

      // 循环解析完整帧
      while (buffer.length >= 4) {
        const view = new DataView(
          buffer.buffer,
          buffer.byteOffset,
          buffer.byteLength,
        );
        const length = view.getInt32(0, false); // Big-Endian Read

        if (buffer.length < 4 + length) {
          break; // 数据不够一帧，等待更多数据
        }

        // 提取一帧数据
        const frameData = buffer.subarray(4, 4 + length);
        buffer = buffer.subarray(4 + length);

        const type = new DataView(
          frameData.buffer,
          frameData.byteOffset,
          1,
        ).getInt8(0);
        const payloadBytes = frameData.subarray(1);

        this.handleFrame(type, payloadBytes);
      }
    }
  }

  // 2. 发送帧至 Deno.stdout
  public async sendFrame(type: number, payload: object) {
    const jsonStr = JSON.stringify(payload);
    const payloadBytes = new TextEncoder().encode(jsonStr);

    const frameLength = 1 + payloadBytes.length;
    const buf = new Uint8Array(4 + frameLength);
    const view = new DataView(buf.buffer);

    // 写入 Header
    view.setInt32(0, frameLength, false); // Length (Big-Endian)
    view.setInt8(4, type); // Type
    buf.set(payloadBytes, 5); // Payload

    await Deno.stdout.write(buf);
  }

  private handleFrame(type: number, payloadBytes: Uint8Array) {
    const json = JSON.parse(new TextDecoder().decode(payloadBytes));
    // 路由逻辑: 请求/响应/事件分发...
  }
}
```

#### (2) 暴露给插件开发者的 SDK API

```typescript
// index.ts - 插件入口文件示例
import { ora } from "@ora-space/sdk";

// 监听生命周期事件
ora.onActivate(async () => {
  // 调用宿主暴露的 Capability API
  const weather = await ora.fetch("https://api.weather.com/v1");

  // UI 交互
  await ora.ui.showNotification({
    title: "Weather Updated",
    body: `Current temp: ${weather.temp}°C`,
  });
});
```

### 3. 标准输出与日志隔离 (Stdio Isolation)

`Deno.stdout` 被二进制 IPC 独占后，`Deno.stderr` 承担起日志输出的重任。为了实现类似 VS Code Output Channel 的日志分级与精准追溯，不能简单以文本打印，必须使用结构化 Log Chunk 协议。

在 SDK 启动时，全局替换 console.log / console.error 等方法，并绑定为结构化 Logger。如果插件内部发生未捕获的 Error 或 Promise Rejection，SDK 捕获后同步输出 Traceback 到 stderr 并通过 IPC 告知宿主。

在二进制 Stdio 通信与 SDK 解包层打通后，SDK 的能力建设就是**连接 TypeScript 插件代码与 Rust 宿主的枢纽**。

按照 VS Code Extension API 的现代设计理念，SDK 的能力建设需要围绕四个关键维度展开：**权限安全代理 (Proxy & Interception)**、**日志与可观测性 (VS Code 风格 Logging)**、**生命周期与状态持久化**，以及 **RPC/事件流基础设施**。

---

### 1. 结构化日志系统 (VS Code 风格 Alignment)

`Deno.stdout` 被二进制 IPC 独占后，`Deno.stderr` 承担起日志输出的重任。为了实现类似 VS Code Output Channel 的日志分级与精准追溯，不能简单以文本打印，必须使用**结构化 Log Chunk 协议**。

```typescript
// @ora-space/sdk/src/logger.ts
export enum LogLevel {
  Trace = 0,
  Debug = 1,
  Info = 2,
  Warn = 3,
  Error = 4,
  Off = 5,
}

export class Logger {
  private pluginId: string;
  public level: LogLevel = LogLevel.Info;

  constructor(pluginId: string) {
    this.pluginId = pluginId;
  }

  private formatAndWrite(levelStr: string, args: any[]) {
    const timestamp = new Date().toISOString();
    const message = args
      .map((arg) =>
        typeof arg === "object" ? JSON.stringify(arg) : String(arg),
      )
      .join(" ");
    // 按照 VS Code 规范的控制台对齐输出格式: [YYYY-MM-DD HH:mm:ss.SSS] [PluginID] [Level] Message
    const formatted = `[${timestamp}] [${this.pluginId}] [${levelStr}] ${message}\n`;
    Deno.stderr.writeSync(new TextEncoder().encode(formatted));
  }

  info(...args: any[]) {
    if (this.level <= LogLevel.Info) this.formatAndWrite("INFO ", args);
  }
  warn(...args: any[]) {
    if (this.level <= LogLevel.Warn) this.formatAndWrite("WARN ", args);
  }
  error(...args: any[]) {
    if (this.level <= LogLevel.Error) this.formatAndWrite("ERROR", args);
  }
  debug(...args: any[]) {
    if (this.level <= LogLevel.Debug) this.formatAndWrite("DEBUG", args);
  }
}
```

**SDK 自动劫持逻辑：**

在 SDK 启动时，全局替换 `console.log` / `console.error` 等方法，并绑定为结构化 `Logger`。如果插件内部发生未捕获的 Error 或 Promise Rejection，SDK 捕获后同步输出 Traceback 到 stderr 并通过 IPC 告知宿主。

---

### 2. 运行时权限代理层 (Capability Proxying)

虽然 Deno 在启动时通过 Flag（如 `--allow-net`）行使底层的硬隔离，但有些高级 Capability（例如操作 UI 弹窗、访问专属 SQLite 数据库、注册系统快捷键）无法被 Deno CLI 原生控制。

SDK 内部建立了一套 **Capability Guard 代理机制**，在 TypeScript 触发底层 API 前进行拦截与语义校验：

```
+-------------------------------------------------------------------+
|                        Plugin Code (TS)                           |
+-------------------------------------------------------------------+
                                 │ ora.http.fetch()
                                 v
+-------------------------------------------------------------------+
|                     @ora-space/sdk Barrier                        |
| 1. 检查 orax.toml 校验该 API 是否受申明 Capability 许可              |
| 2. 构造标准的 JSON-RPC 2.0 请求 (FrameType = 0x01)                 |
+-------------------------------------------------------------------+
                                 │ Binary Frame (stdin/stdout)
                                 v
+-------------------------------------------------------------------+
|                        Ora Host (Rust)                            |
| 1. Token/Capabilities 运行时二次校验                              |
| 2. 执行真正的网络请求 / 系统 API 调用                              |
+-------------------------------------------------------------------+

```

#### API 设计划分：

- **`ora.http`**：提供受到 Capability 约束的网络请求包装器。
- **`ora.ui`**：控制 Toast 提示、对话框、快捷指令列表菜单、自定义 UI 视图面板（Panel）。
- **`ora.storage`**：持久化数据存取。
- **`ora.workspace`**：监听用户在 Ora 中的选中文本、触发菜单命令等。

---

### 3. 持久化数据与状态 API (`ora.storage`)

在之前设计的目录划分中，插件拥有专属的 `~/.ora/plugins/data/<plugin_id>` 目录。SDK 提供两套不同的存储能力：

#### (1) 轻量级 Key-Value 存储 (`ora.storage.globalState` / `workspaceState`)

类似 VS Code Extension Context 的状态持久化 API：

```typescript
export interface Storage {
  get<T>(key: string, defaultValue?: T): Promise<T | undefined>;
  update(key: string, value: any): Promise<void>;
  delete(key: string): Promise<void>;
}
```

_底层实现_：SDK 将更新请求序列化为 JSON-RPC 消息送往 Rust 宿主，由 Rust 在本地维护 `data/state.json` 或嵌入式 KV (如 RocksDB/Sled) 的写穿（Write-Through）。

#### (2) 私有文件访问 (`ora.storage.fs`)

如果插件声明了 `fs.read` / `fs.write` Capability，SDK 允许插件使用基于 Deno 原生 API 的包装层直接在 `data/` 专属目录下读写大文件。

---

### 4. 生命周期管理与 Event Loop Hook

为了确保插件生命周期（Activating -> Active -> Deactivating -> Disposed）的受控与干净清理，SDK 定义了一套标准 Context：

```typescript
export interface Disposable {
  dispose(): void | Promise<void>;
}

export interface ExtensionContext {
  readonly pluginId: string;
  readonly storagePath: string;
  readonly globalState: Storage;

  // 注册需要清理的资源 (事件监听器、定时器、临时文件)
  subscriptions: Disposable[];
}

export type ActivateFunction = (ctx: ExtensionContext) => void | Promise<void>;
export type DeactivateFunction = () => void | Promise<void>;
```

#### 生命周期事件流动：

1. **宿主触发启动**：Rust 通过 Stdio 发送 JSON-RPC 通知 `ora/activate`。
2. **SDK 准备 Context**：初始化 `ExtensionContext`，执行插件导出的 `activate(ctx)` 函数。
3. **安全卸载 (Graceful Teardown)**：

- 当用户禁用插件或 Ora 准备关闭时，Rust 发送 `ora/deactivate`。
- SDK 触发 `deactivate()` 钩子，设置 Timeout（例如 2 秒）。
- SDK 遍历 `ctx.subscriptions` 数组，依次调用每个 `Disposable.dispose()` 清理挂载的事件与资源。

---

### 5. 整体 SDK API 架构呈现

最终，提供给插件开发者的完整能力图谱如下：

```typescript
import { ora, ExtensionContext, Disposable } from "@ora-space/sdk";

export async function activate(ctx: ExtensionContext) {
  // 1. 日志记录
  ora.logger.info("Ora Weather Plugin is activating...");

  // 2. 读取持久化配置
  const cachedCity = await ctx.globalState.get("selected_city", "Beijing");

  // 3. 注册 Command (当用户在 UI 搜寻框搜索 Weather 时触发)
  const commandDisposable = ora.commands.registerCommand(
    "weather.query",
    async (args) => {
      const data = await ora.http.fetch(
        `https://api.weather.com/v1?city=${cachedCity}`,
      );

      await ora.ui.showToast({
        type: "success",
        message: `Current temp in ${cachedCity}: ${data.temp}°C`,
      });
    },
  );

  // 4. 纳管到清理列表
  ctx.subscriptions.push(commandDisposable);
}

export async function deactivate() {
  ora.logger.info("Cleaning up resources...");
}
```

---

### 接下来建议讨论的焦点

确定了 `deno run --no-prompt` 的总体方案后，我们有以下几个细化方向：

1. **IPC 通信与 SDK 注入**：Ora 主进程与 Deno 子进程如何高效通信（stdio JSON-RPC v.s. Domain Socket/Named Pipe）？插件端如何获得 `ora` API 对象（如注入全域 SDK）？
2. **`fs.data` 的绝对路径解析**：在插件启动前，Ora 如何动态解析并创建好 `~/.ora/plugins/data/<plugin_id>` 目录，并注入为相对路径环境变量？
3. **开发与调试模式 (`--head` / Hot Reload)**：在开发模式下，如何支持 `deno run --watch` 或是热重载插件？

你打算先深入交流哪个部分？

### 4. 运行与加载 (Runtime & Loading)

- **加载机制 (Loading Strategy)：**
- _惰性加载 (Lazy Loading / On-Demand)_：用到命令或 IPC 调用时才拉起进程。
- _预加载 (Eager / Startup Loading)_：随 Ora 主程序启动即被初始化。

每个类型插件用 bitmap 去控制可以配置的 hook 点。fs net proc 需要额外配置。

- **隔离与沙箱 (Isolation & Sandboxing)：** 插件运行的隔离级别（进程隔离 IPC / WASM 沙箱 / Webview 视图隔离）。
- **上下文注入 (Context Injection)：** 宿主向插件注入 API 句柄、环境变量、运行期 Config 等。

### 6. 配置与持久化 (Configuration & Data)

- **插件设置 (Plugin Settings / Preferences)：** 宿主统一为插件提供配置面板展示与 Key-Value/TOML 级别的配置存储。
- **数据隔离 (Data Isolation / Storage Area)：** 为每个插件分配专属的运行数据目录（如 `~/.ora/data/plugins/<plugin-id>/`），防止插件乱写本地磁盘。
-
