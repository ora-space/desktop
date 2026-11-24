# 更新日志

[English](release-notes.md) | 中文

每个正式版本在本仓库有一份手写的 `release-notes/<版本>.json`，它是该版本日志的唯一来源。格式和检查归
`scripts/release-notes.ts` 所有。

## 写日志

发版前，描述上一个正式版 tag 到准备打包的提交之间用户能感知的改动。可以用本地 AI 起草，由人改好后提普通 PR。打 tag 前核对
该 PR 之后合并的提交，补上漏写的内容，tag 打在核对过的提交上。

```json
{
  "schema": 1,
  "version": "0.7.0",
  "date": "2026-12-10",
  "entries": [{ "category": "feature", "zh-CN": "……", "en-US": "…" }]
}
```

| 字段                                 | 规则                                                            |
| ------------------------------------ | --------------------------------------------------------------- |
| `schema`                             | `1`                                                             |
| `version`                            | 不带 `v` 的正式版本号，等于文件名                               |
| `date`                               | 计划发布日期，`YYYY-MM-DD`，可能与实际发布日不同                |
| `entries`                            | 至少一条                                                        |
| `entries[].category`                 | `important`、`feature`、`improvement`、`fix` 之一               |
| `entries[].zh-CN`、`entries[].en-US` | 都必填，单行纯文本，最多 500 个字符，不含控制字符和双向格式字符 |

不接受其他字段；日志不列来源 PR 和提交范围。

## 检查

工具脚本的测试会校验仓库里的每份日志，只改 `release-notes/` 的 PR 也会触发。

`task package:desktop -- --tag <版本>` 和发布工作流只在满足下列条件时构建：

- `release-notes/` 里有该正式版本的日志。`v0.7.0`、`v0.7.0-rc.1` 和 `0.7.0+suffix` 都要求 `0.7.0.json`。
- 每份日志都合格，不只是本版本的那份。
- `release-notes/` 与检出的提交一致：没有修改、删除、未跟踪或被忽略的文件，并且源码是 git 检出。其他位置的改动不影响。

不带 `--tag` 的打包跳过这些检查。
