# Deferred Controller–Node logging

English | [中文](controller-node-logging.zh.md)

Tracking: `todo-87602f0b`. No logging calls or logging behavior tests are implemented in this slice.
Every location below has an English TODO in `crates/node-protocol/src/frame.rs`. The shared generic
codec is the single recording site for both peer directions; wrappers must not duplicate events.
Future implementation must carry direction metadata into these generic sites and add typed-value
formatting bounds if needed. Typed messages must only be diagnosed at TRACE.

| Expected event                        | Level | Source TODO location                             | Available fields                                             |
| ------------------------------------- | ----- | ------------------------------------------------ | ------------------------------------------------------------ |
| Accepted inbound message              | TRACE | `read_message`, after `message.validate()`       | Typed value, payload length, known frame type                |
| Successfully written outbound message | TRACE | `write_message`, after successful `write_frame`  | Typed value, payload length, frame type                      |
| Inbound semantic rejection            | WARN  | `read_message`, `message.validate()?`            | Validation category/fields, payload length, frame type       |
| Outbound semantic rejection           | WARN  | `write_message`, `message.validate()?`           | Validation category/fields; no serialized length             |
| JSON decode failure                   | WARN  | `read_message`, `from_slice(...).map_err(...)`   | Payload length, frame type, JSON category/location           |
| JSON encode failure                   | ERROR | `write_message`, `to_vec(...).map_err(...)`      | Serializer category; no complete payload length              |
| Clean EOF                             | TRACE | `read_frame`, initial `UnexpectedEof` arm        | Receive direction; no frame metadata                         |
| Initial header I/O failure            | WARN  | `read_frame`, initial error arm                  | I/O kind; no complete length/type                            |
| Remaining length-header I/O failure   | WARN  | `read_frame`, first `read_exact`                 | First byte, I/O kind                                         |
| Invalid inbound length                | WARN  | `read_frame`, range rejection                    | Length, limit, InvalidLength                                 |
| Type-header I/O failure               | WARN  | `read_frame`, second `read_u8`                   | Length, I/O kind                                             |
| Unknown frame type                    | WARN  | `read_frame`, type rejection                     | Length, type, UnsupportedFrameType                           |
| Payload read I/O failure              | WARN  | `read_frame`, payload `read_exact`               | Length, type, expected payload length, I/O kind              |
| Invalid outbound length               | WARN  | `write_frame`, checked length calculation        | Payload length, limit, type, InvalidLength                   |
| Length conversion failure             | ERROR | `write_frame`, `u32::try_from(...).map_err(...)` | Length, type, InvalidLength; unreachable under current limit |
| Length-header write I/O failure       | WARN  | `write_frame`, first `write_all`                 | Length, type, I/O kind                                       |
| Type-header write I/O failure         | WARN  | `write_frame`, `write_u8`                        | Length, type, I/O kind                                       |
| Payload write I/O failure             | WARN  | `write_frame`, payload `write_all`               | Length, type, payload length, I/O kind                       |
| Flush I/O failure                     | WARN  | `write_frame`, `flush`                           | Length, type, I/O kind                                       |

All rows also record send/receive direction. Shared callers propagate framing and I/O errors without
recording them again. Partial I/O buffers must not be presented as complete messages, and a write or
flush failure does not undo previously written bytes. Later work must use `ora_logging` and verify
structured events and isolation from the protocol byte stream.
