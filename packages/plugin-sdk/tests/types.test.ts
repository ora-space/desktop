import assert from "node:assert/strict";
import test from "node:test";

import type {
  PluginJsonRpcErrorResponse,
  PluginJsonRpcRequest,
  PluginJsonRpcSuccessResponse,
} from "../src/index.js";

test("exports JSON-RPC protocol DTOs with JSON number fields", () => {
  const request = {
    jsonrpc: "2.0",
    id: "1",
    method: "add",
    params: {
      a: 1,
      b: 2,
    },
  } satisfies PluginJsonRpcRequest;
  const successResponse = {
    jsonrpc: "2.0",
    id: request.id,
    result: 3,
  } satisfies PluginJsonRpcSuccessResponse;
  const errorResponse = {
    jsonrpc: "2.0",
    id: request.id,
    error: {
      code: -32601,
      message: "missing method",
    },
  } satisfies PluginJsonRpcErrorResponse;

  assert.equal(JSON.stringify(request), "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"add\",\"params\":{\"a\":1,\"b\":2}}");
  assert.equal(successResponse.result, 3);
  assert.equal(errorResponse.error.code, -32601);
});
