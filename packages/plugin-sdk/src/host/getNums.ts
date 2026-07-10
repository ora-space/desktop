import { readLine } from "../internal/reader";

/** Generic JSON-RPC request read from the transport — intentionally method-agnostic. */
interface JsonRpcRequest {
  jsonrpc: string;
  id: string;
  method: string;
  params: unknown;
}

/**
 * 从 stdin 读取一条 Host 发来的请求。
 *
 * 内部流程:
 *   1. 调用 readLine() 获取一行文本
 *   2. JSON.parse 解析
 *   3. 校验 jsonrpc === "2.0" 且有 id 和 method 字段
 *   4. 组装为 JsonRpcRequest 返回
 *
 * 返回 null 表示 stdin 已关闭。
 * 收到不合法的行（非 JSON、缺少字段）会跳过并继续读取下一行。
 */
export async function getNums(): Promise<JsonRpcRequest | null> {
  while (true) {
    const line = await readLine();
    if (line === null) return null;

    try {
      const obj = JSON.parse(line);
      if (
        obj.jsonrpc === "2.0" &&
        typeof obj.id === "string" &&
        typeof obj.method === "string"
      ) {
        return {
          jsonrpc: obj.jsonrpc,
          id: obj.id,
          method: obj.method,
          params: obj.params ?? null,
        };
      }
      // 不合法的消息，跳过，继续读下一行
    } catch {
      // 非 JSON，跳过，继续读下一行
    }
  }
}
