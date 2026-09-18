# 受管仓库 Clone

[English](repository-clone.md) | 中文

Linux Node 为嵌入调用方提供 `configure_clone`、`submit_clone` 和 `recover_clones`。
独立可执行程序接受可选 `clone` 部署配置并恢复已受理的 clone；不通过文件、stdin 或对外 IPC
接收新命令，也未切换 Backend 写入入口。

```json
{
  "clone": {
    "repository_root": "/home/node/repositories",
    "git_config": "/home/node/deployment/clone.gitconfig",
    "search_path": ["/usr/bin"],
    "ssh": { "kind": "disabled" }
  }
}
```

这是原独立入口配置的附加部分，不是完整配置文件。启动前须部署根目录和配置文件。路径必须绝对，
由当前用户或 root 控制，禁止 group／other 写入及符号链接路径分量。Node 不修改部署权限。
根目录不能与 Node／process 状态或已配置的 Worktree checkout 重叠。文件系统须支持 Unix birth time：
使用设备、inode、创建时间识别根和新建目标。身份缺失或替换保持 Unknown，数据库预留本身不证明目录归属。
这是无需 root、信任同用户的保护，不是对抗同 UID 恶意代码的隔离。

显式 Git 配置可指定 CA 证书和非交互部署凭据 helper。Run 环境只保存配置路径，不继承 Worktree 环境
或环境 HOME。Clone 输出直接丢弃，认证诊断不会进入 journal 输出。
SSH 使用 `{"kind":"configured","program":"/usr/bin/ssh","config":"/home/node/deployment/ssh_config"}`；
部署文件指定身份和 known hosts，Node 强制 batch mode 与严格主机密钥检查。未知主机密钥由部署处理，
Node 不交互接受。

每次受理独占新建目录，每个执行最多一个持久变更 Run。Git 执行非本地、完整历史、单分支 clone，
禁用 template、递归子模块、LFS smudge／process filter、hooks 和交互提示。同名 tag 不能满足分支请求。
核实普通非 shallow checkout、独立对象、原来源／分支、HEAD 与已获取远端分支 commit 相同及 tracked 文件干净。

恢复先关闭原受管 scope，再核实退出和原生目录证据，最后读取仓库事实。进程证据缺失不允许新 Run。
没有 Run 的预留，仅在持久阶段证明从未派发时才可继续。已知失败保留目录；重试必须使用新 operation／execution，
获得新目录。终态重放只使用原持久结果和事件，即使离线或文件被用户编辑，也不再检查或 clone。

真实 Linux host／guardian 测试覆盖 TLS clone、部署凭据、分支缺失／仅有 tag、认证失败、checkout 扩展禁用、
结果重放、已存在／替换路径、获取期间 Node 强杀、SSH 未知主机拒绝／成功及终态／outbox 写入失败恢复。
Linux 验收需安装 OpenSSH client／server 并由系统预备 `/run/sshd`；fixture daemon 以普通测试用户在临时端口运行。
CI 部署的是测试依赖，不是生产 Node 服务。这不代表 Controller 投递／重连验收或跨平台 Node 运行时完成。

参见[持久记录](persistence/repository-acquisition.zh.md)和[运行时责任](runtime.zh.md)。
