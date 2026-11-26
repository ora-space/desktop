# Skill 插件配置

[English](skill-configuration.md) | 中文

Skill 插件可以用 Settings-only `assets/config.json` 声明配置。现有插件配置编辑器把覆盖值存入宿主插件数据目录；配置不改变 Skill Catalog 发布与会话清单交付。

```json
{
  "schemaVersion": 1,
  "settings": {
    "endpoint": {
      "type": "string",
      "title": "Endpoint",
      "description": "服务地址",
      "default": "https://example.test"
    },
    "token": {
      "type": "string",
      "title": "Token",
      "description": "服务凭据",
      "required": true,
      "secret": true
    }
  }
}
```

`secret` 默认 false，支持字符串、数字和布尔字段。编辑器响应隐藏敏感值及默认值，保存时保留未修改的敏感字段。“重置此项”删除存储覆盖，“全部重置”恢复默认值。MCP 不论此标记如何设置都继续全量脱敏。不要把真实凭据写进发布包的默认值或说明。

## 脚本读取

Desktop 启动时冻结已安装有效 Skill 插件的完整配置。所有 Agent 插件进程和宿主代启动的子进程共用快照，包括不同工作区。保存、重置、安装、更新、卸载均在重启 **Ora Desktop** 后影响注入；仅新建对话或重启 Agent 不会刷新。编辑器会显示提示。

变量名为 `ORA_SKILL_<PLUGIN>_<FIELD>`。完整插件 ID（含 namespace）和字段 ID 中的 ASCII 字母数字转大写，其余每个 Unicode 字符各替换为一个 `_`；连续 `_` 不合并。例如 `official/demo` 的 `token` 为 `ORA_SKILL_OFFICIAL_DEMO_TOKEN`。Python 示例：

```python
import os

token = os.environ.get("ORA_SKILL_OFFICIAL_DEMO_TOKEN")
if token is None:
    raise RuntimeError("请配置 demo 插件并重启 Ora Desktop")
# 将 token 交给服务客户端，不要输出它。
```

字符串原样交付，数字采用 JSON 表示，布尔值为 `true`／`false`。缺失的可选值不注入。不完整、非法或不可读的配置，以及包含 NUL 的值，导致该插件整包不注入。Skill 文件仍可用，不影响其他 Agent 和插件。

建快照时先对有效声明的全部字段不区分大小写检测撞名，包括同插件字段和跨插件。可选值缺失，以及配置值缺失、损坏、非法或含 NUL 的插件仍参与检测；插件或声明本身无效时不参与。名字收集后再判断值是否具备注入资格；涉及的插件整包不注入，诊断只含插件和字段 ID，不含配置值。该前缀归宿主所有：继承环境和 spawn 调用方 env 中大小写不敏感匹配的变量先移除，再注入快照；不拒绝整个 spawn。不支持自定义别名，声明 `env: GITHUB_TOKEN` 等未知成员会被拒绝。

## 安全边界

配置仍保存在宿主 `store.json`，本功能没有新增加密或系统钥匙串。Ora 不把配置值复制进工作区文件、ACP 消息或宿主日志。公开字段在编辑器中仍然可见。

每个 Agent 都能读取整份 Desktop 快照，**不提供工作区或 Skill 级凭据隔离**。Agent 和脚本可以自行读取并输出环境值，脱敏不能阻止恶意脚本泄漏。重置或卸载后，已有进程终止前仍持有旧快照；需要立即撤销时，应在外部服务吊销凭据。

本契约仅覆盖 Desktop 安装的 Skill 插件，不覆盖普通导入 Skill、Cloud 或远程执行。声明 `secret` 的包要求宿主支持此字段，旧版本的严格解析器可能拒绝新包。

安全承诺只覆盖宿主自己的交付路径；脚本自行输出的值会进入 ACP 消息、插件日志、debug TRACE 日志和会话 JSONL。
