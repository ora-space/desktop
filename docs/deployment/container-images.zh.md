# Node 与 Controller 容器镜像

[English](container-images.md) | 中文

`docker/Dockerfile` 用一次 workspace release 构建产出两个镜像，两个 target 一起构建只编译一次：

```sh
docker build -f docker/Dockerfile --target node -t ora-node:local .
docker build -f docker/Dockerfile --target controller -t ora-controller:local .
```

`ora-space/cluster` 的本地整套环境（`task up`）会构建这两个镜像。运行时基于 Debian bookworm，构建阶段
使用与 `rust-toolchain.toml` 相同的编译器。

## Node 镜像

每个沙盒一个容器，由 Sandbox Server 启动，Compose 从不直接启动它。与 Sandbox Server 的约定：

| 输入                       | 规则                                                                                               |
| -------------------------- | -------------------------------------------------------------------------------------------------- |
| `ORA_NODE_CONFIG`          | 完整的 Node 服务配置 JSON；为空则拒绝启动。                                                        |
| `/var/lib/ora`             | Workspace 卷的挂载点。配置中的所有状态路径（Node home、process host 目录、clone 根目录）都在其下。 |
| `/etc/ora/clone.gitconfig` | clone 使用的 Git 配置；镜像自带一个不含凭据、属主为 root 的空文件。                                |
| 用户                       | 服务以 `node`（UID 1000）运行，与 `process.expected_uid` 一致。                                    |

入口脚本由 `tini` 作为 PID 1 托管。它以 root 身份只做三件事：把卷根目录设为 `node` 所有、权限 `0700`
（不改动其中内容），创建 Node 要求预先存在的 clone 根目录，把配置写入 `/run/ora/node.json`；然后降权
为 `node`。host 目录不存在时以 `create` 启动 process host，否则以 `recover` 启动，因此同一个 Workspace
卷在多个容器之间沿用同一份 host 日志；恢复失败不会退回为新建。等 host socket 真正可连接（残留的 socket
文件不算）后才启动 `ora-node`。

停止信号先交给 Node，由它经仍在运行的 host 关闭受管 scope；Node 退出后才停止 host。guardian 不会被
停止：它们的日志留在卷上，由下一个容器恢复。因此 Sandbox Server 的停止超时必须大于 Node 的
`shutdown_grace_ms + cleanup_timeout_ms`。Node 自行退出时同样停止 host，容器以 Node 的退出码退出。

可执行文件位于属主为 root 的 `/opt/ora/bin`，release 构建的 host 要求 guardian 位于这样的目录。

## Controller 镜像

以非特权用户 `controller` 运行 `ora-controller`，私有 home 目录为 `/var/lib/ora-controller`。部署方挂载
配置文件并传入 `--config`；云端持久模式下进程不提供监听器，因此镜像不暴露端口。配置见
[本机 Controller 运行时](../controller/local-runtime.zh.md)。
