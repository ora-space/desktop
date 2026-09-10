# 延后的 Controller–Node 日志

[English](controller-node-logging.md) | 中文

跟踪项：`todo-87602f0b`。本阶段尚未实现日志调用或日志行为测试。
下表每个位置在 `crates/node-protocol/src/frame.rs` 中都有一个英文 TODO。共享的通用 codec
是两个对端方向唯一的记录位置；包装层不得重复记录事件。后续实现必须将方向元数据传入这些
通用位置，并在需要时增加 typed value 的格式化约束。typed message 只能在 TRACE 级别诊断。

| 预期事件                  | 级别  | 源 TODO 位置                                     | 可用字段                                      |
| ------------------------- | ----- | ------------------------------------------------ | --------------------------------------------- |
| 接收消息已接受            | TRACE | `read_message`，`message.validate()` 之后        | 类型化值、payload 长度、已知 frame 类型       |
| 发送消息已成功写入        | TRACE | `write_message`，`write_frame` 成功之后          | 类型化值、payload 长度、frame 类型            |
| 接收消息语义拒绝          | WARN  | `read_message`，`message.validate()?`            | 校验类别／字段、payload 长度、frame 类型      |
| 发送消息语义拒绝          | WARN  | `write_message`，`message.validate()?`           | 校验类别／字段；没有序列化长度                |
| JSON 解码失败             | WARN  | `read_message`，`from_slice(...).map_err(...)`   | payload 长度、frame 类型、JSON 类别／位置     |
| JSON 编码失败             | ERROR | `write_message`，`to_vec(...).map_err(...)`      | 序列化器类别；没有完整 payload 长度           |
| 正常 EOF                  | TRACE | `read_frame`，初始 `UnexpectedEof` 分支          | 接收方向；没有 frame 元数据                   |
| 初始 header I/O 失败      | WARN  | `read_frame`，初始错误分支                       | I/O 类型；没有完整长度／类型                  |
| 剩余长度 header I/O 失败  | WARN  | `read_frame`，第一次 `read_exact`                | 第一个字节、I/O 类型                          |
| 接收长度无效              | WARN  | `read_frame`，范围拒绝                           | 长度、限制、`InvalidLength`                   |
| 类型 header I/O 失败      | WARN  | `read_frame`，第二次 `read_u8`                   | 长度、I/O 类型                                |
| 未知 frame 类型           | WARN  | `read_frame`，类型拒绝                           | 长度、类型、`UnsupportedFrameType`            |
| payload 读取 I/O 失败     | WARN  | `read_frame`，payload `read_exact`               | 长度、类型、预期 payload 长度、I/O 类型       |
| 发送长度无效              | WARN  | `write_frame`，检查长度计算                      | payload 长度、限制、类型、`InvalidLength`     |
| 长度转换失败              | ERROR | `write_frame`，`u32::try_from(...).map_err(...)` | 长度、类型、`InvalidLength`；当前限制下不可达 |
| 长度 header 写入 I/O 失败 | WARN  | `write_frame`，第一次 `write_all`                | 长度、类型、I/O 类型                          |
| 类型 header 写入 I/O 失败 | WARN  | `write_frame`，`write_u8`                        | 长度、类型、I/O 类型                          |
| payload 写入 I/O 失败     | WARN  | `write_frame`，payload `write_all`               | 长度、类型、payload 长度、I/O 类型            |
| flush I/O 失败            | WARN  | `write_frame`，`flush`                           | 长度、类型、I/O 类型                          |

所有行还会记录发送／接收方向。共享调用方会传递 framing 和 I/O 错误，不会再次记录这些错误。
部分 I/O 缓冲区不得作为完整消息呈现；写入或 flush 失败不会撤销已经写出的字节。后续工作必须
使用 `ora_logging`，并验证结构化事件以及日志与协议字节流之间的隔离。
