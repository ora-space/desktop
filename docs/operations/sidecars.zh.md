# Sidecar

[English](sidecars.md) | 中文

Desktop 的 release 构建在可执行文件旁边打包 Deno 和 ripgrep，Node 镜像自带 Deno。两者在使用前都根据审核过的固定值校验。`ora-reaper`
sidecar 则由 `task build:desktop` 从检出的源码构建。

## 校验

`scripts/setup-binary.ts` 在 Desktop 前端构建之前运行。它根据所选 release 的可执行文件 SHA-256 检查
`apps/desktop/src-tauri/binaries` 中的 Deno 和 ripgrep，对本机目标还会运行 `--version`。过期、损坏或无法执行的文件只有在新下载
通过所有检查之后才会被替换；其他目标平台的可执行文件只通过摘要认证，不运行。

`scripts/sidecar-checksums.json` 是信任来源：它的 key 把精确的官方 GitHub release URL 绑定到 Linux 和 Windows x64（以及
Linux arm64 的 Deno）的压缩包和可执行文件 SHA-256。Node 镜像从同样的固定值安装 Deno。

- 每次安装都在 `binaries` 内、目标文件系统上自己的临时目录中暂存。压缩包在解压之前必须匹配固定值；解压出的普通可执行文件必须
  匹配固定值，在本机上还必须报告期望的版本，然后才通过原子重命名发布。
- 失败时保留之前的二进制并让构建失败。清理只删除本次安装的暂存目录，并发的安装从不共享暂存目录，已有的文件和旧的 `.extract-*`
  目录既不复用也不删除。
- 未知的 release 或目标平台、缺失的固定值和格式错误的摘要都以关闭方式失败。没有校验和覆盖，也没有未经校验的后备；`--force` 只
  跳过缓存复用。

`deno task setup:binary` 接受 `deno` 或 `rg` 来只安装一个 sidecar。debug 版的 Desktop 构建使用 `PATH` 中的 `deno` 和 `rg`，见
[Desktop 宿主](../desktop/desktop-host.zh.md#内置-sidecar)。

## 更新 sidecar

1. 选择官方 release。Deno 要更新 `.deno-version` 和 `package.json` 的 `engines.deno`（`deno task check:toolchain` 会检查两者）；
   ripgrep 要更新 setup 和 CI 中的版本。
2. 从官方仓库的 GitHub release API（`/repos/denoland/deno/releases/tags/v<VERSION>` 或
   `/repos/BurntSushi/ripgrep/releases/tags/<VERSION>`）读取 release 资产的摘要，有发布的校验和资产时与之比较。
3. 下载每个受支持的压缩包，校验它的 SHA-256，解压，计算 `deno`/`rg` 可执行文件（Windows 上是 `.exe`）的哈希，并在精确的
   release URL 下更新两个哈希。
4. 审查固定值的改动，运行 `deno task test:tooling`，用 `deno task setup:binary --force` 实际走一遍下载，并在受支持的平台上运行
   Desktop 构建 CI。
