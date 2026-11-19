# 子代理详情

[English](subagent-details.md) | 中文

父对话可以展示 OpenCode 内联子会话与 CodeAgent `_subagent/update` 子代理的只读卡片。展开后查看文本、思考、计划和工具输入/输出；OpenCode 嵌套卡片依据
显式父子关系连接，而 `_subagent/update` 不携带子任务间父子链，嵌套子代理统一按 `depth: 1` 展平为同级卡片。父回复仍在普通时间线上。原有停止入口控制父 prompt，卡片不新增单独取消、steer 或 close。

父回合仍在观察期间，没有受支持的 provider 状态报告就显示未上报。父回合完成、失败或取消后，只有当
同一父回合、同一来源流中最晚一次委托调用满足下列之一时，卡片才显示**委托已返回结果**：（a）OpenCode 委托在
`rawOutput.metadata.sessionID` 中指向该子组、自身为 ACP `completed` 且 provider `status` 为 `completed`；或
（b）`_subagent/update` 显式关联的父 `toolCallId` 在父流中由 provider 报告 `completed`、带有工具输出（`content` 或 `rawOutput`）且未带非 `completed` 的 `rawOutput.status` / `rawOutput.metadata.status`（回合结束补成的完成不计）。其余情况——委托未完成、
失败、取消、后台 `status=running`，或没有可关联的委托标识——显示**后续未知（投影已停止）**。最晚一次
按持久化位置判断，后续回合续接同一子会话不会改写先前的回合。两种文案都只表示前台委托已返回结果或后续
不可见，不代表子代理已完成、取消或停止；实时、暖加载和冷加载由同一批持久化事实得出相同文案。旧历史仅到
EOF、没有 turn_end 时保持未知，不补造父结束事实。父回合结束时，未上报终态的子工具显示“结果未知”。
父关系缺失、冲突或循环时，内容仍作为独立卡片可读，且不据此声称委托已返回。

来源只读取 `_meta["opencode/child-session"]` 或由 ACP `_subagent/update` 扩展通知持久化的 `_meta["_subagent/update"]`，不根据工具标题、ID 前缀或 depth 猜测。未支持或无效的来源
继续显示在父时间线。实时批处理与重放保留完整 metadata 和文本注解，不因 messageId 相同或缺失而混合
不同来源；工具增量只替换实际给出的顶层命名空间，包括显式 null，不递归合并未知 provider 对象。

[历史记录模块](session-history.zh.md)必须保存来源 metadata。界面无法补回旧文件中丢失的来源，OpenCode
普通投影也可能在父 prompt 结束后停止转发。因此卡片明确只展示已收到内容，不承诺完整后台历史。
实时活动观察与历史内容是两个不同的概念。
