# 容器镜像

[English](container-images.md) | 中文

`docker/Dockerfile` 从同一次 workspace release 构建产出 Node 和 Controller 两个镜像，所以构建两者只编译一次。`ora-space/cluster`
中的本地环境会构建并运行这两个镜像。

```sh
docker build -f docker/Dockerfile --target node -t ora-node:local .
docker build -f docker/Dockerfile --target controller -t ora-controller:local .
```

两者都使用 Debian bookworm 运行环境；构建阶段使用 `rust-toolchain.toml` 中的编译器。

## Node 镜像

每个 Workspace 沙盒一个容器，由 Sandbox Server 启动（从不由 Compose 启动）。

| 输入 | 规则 |
|---|---|
| `ORA_NODE_CONFIG` | 以 JSON 表示的完整 [Node 配置](../node/node.zh.md)；为空时拒绝启动 |
| `ORA_NODE_CERT`、`ORA_NODE_KEY`、`ORA_NODE_CA`、`ORA_CONTROLLER_CERT` | Node 双向 TLS 控制监听使用的 Base64 PEM，见[控制会话](../node/control-session.zh.md) |
| `ORA_MODEL_CLIENT_CERT`、`ORA_MODEL_CLIENT_KEY`、`ORA_MODEL_CA` | 配置中有 `agent.model_proxy` 时必需 |
| `ORA_GIT_CLIENT_CERT`、`ORA_GIT_CLIENT_KEY`、`ORA_GIT_CA` | 配置中有 `git_access` 时必需 |
| `/var/lib/ora` | Workspace 卷的挂载点；配置中的每个状态路径都在它下面 |
| `/etc/ora/clone.gitconfig` | clone 的 Git 配置；镜像自带一个属于 root、不含凭据的空文件 |

管理进程（Node）以 root 运行，拥有受保护的配置和日志。Git 工作负载以 UID/GID 1000（`node`）运行，这些进程在 fork 之后、
exec 之前切换到这个身份；配置指定了 Agent 工作负载目录时，Agent 插件及其进程也是如此；Node 的网络控制监听要求这个单独的身份。

入口程序在 `tini` 下以 PID 1 运行，所以被重新挂到它下面的进程（Node 被杀后留下的进程）都会被回收。它：

1. 把卷的根目录设为 `root:root`、模式 `0711`（从不修改其中的内容），创建 clone 根目录，配置了 Agent 工作负载目录时也创建它
   （`0711`），Node 需要这个目录但从不创建；
2. 把配置写入 `/run/ora/node.json`，把每个证书和私钥写入 `/run/ora/*.pem`（模式 `0600`，位于只有 root 可访问的目录中），然后
   清除这些环境变量；
3. 启动 `ora-node`，这是它监管的唯一进程：Node 自己运行每一条 Git 命令，并拉起自己的 `ora-reaper`，见
   [Node](../node/node.zh.md#执行与恢复)。

`TERM` 和 `INT` 被转发给 Node，由它结束自己运行的 Git 并等待停止证据。因此 Sandbox Server 的停止超时必须大于 Node 的
`shutdown_grace_ms + cleanup_timeout_ms`。容器以 Node 的退出状态退出。

二进制放在属于 root 的 `/opt/ora/bin` 下；`ora-reaper` 与 `ora-node` 在同一目录，由 `ora-node` 从自己所在的目录启动。镜像还
自带 `/opt/ora/bin/deno`，所有插件都运行在它上面：
版本由 `.deno-version` 指定，构建时根据 `scripts/sidecar-checksums.json`（Linux x64 和 arm64）校验压缩包和可执行文件，没有固定
值的架构会让构建失败。Agent 会话通过 `agent.deno_path` 使用它。

`ora-git` 安装在 workload `PATH` 上的 `/usr/local/bin/ora-git`，Agent 的进程可以直接找到它。它只向当前会话的
`ORA_GIT_SOCKET` 发送固定请求，socket 目录由 `agent.ora_git_directory` 指定；Git Server 凭据只留在 Node。见
[ora-git](../node/ora-git.zh.md)。

## Controller 镜像

以非特权的 `controller` 用户（UID 10001）运行 `ora-controller`，以 `/var/lib/ora-controller` 作为私有 home。部署挂载配置并传入
`--config`。cloud 形态下进程不提供任何监听，所以镜像不暴露端口。见 [Controller](../controller/controller.zh.md)。
