import { writeLine } from "../internal/writer";
import type { PluginJsonRpcErrorResponse } from "../types/plugin-protocol.js";

/** Generic JSON-RPC success response — intentionally method-agnostic. */
interface JsonRpcSuccessResponse {
  jsonrpc: string;
  id: string;
  result: unknown;
}

/**
 * 向 stdout 写入成功响应。
 *
 * @param id     - 对应请求的 id
 * @param result - 返回值，会 JSON.stringify 后写入
 */
export async function returnNums(id: string, result: unknown): Promise<void> {
  const response: JsonRpcSuccessResponse = {
    jsonrpc: "2.0",
    id,
    result: result ?? null,
  };
  writeLine(JSON.stringify(response));
}

/**
 * 向 stdout 写入错误响应。
 *
 * @param id      - 对应请求的 id
 * @param code    - JSON-RPC 错误码
 * @param message - 错误描述
 */
returnNums.error = async function (
  id: string,
  code: number,
  message: string
): Promise<void> {
  const response: PluginJsonRpcErrorResponse = {
    jsonrpc: "2.0",
    id,
    error: { code, message },
  };
  writeLine(JSON.stringify(response));
};
