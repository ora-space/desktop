import { test, expect, mock, beforeEach } from "bun:test";

// Mock readLine — 先 mock 再 import getNums
const readLineMock = mock();
mock.module("../../src/internal/reader", () => ({
  readLine: readLineMock,
}));

const { getNums } = await import("../../src/host/getNums");

beforeEach(() => {
  readLineMock.mockClear();
});

test("正常请求——解析 JSON-RPC 请求为 JsonRpcRequest", async () => {
  readLineMock.mockResolvedValueOnce(
    '{"jsonrpc":"2.0","id":"1","method":"add","params":{"a":1,"b":2}}'
  );

  const result = await getNums();

  expect(result).toEqual({ jsonrpc: "2.0", id: "1", method: "add", params: { a: 1, b: 2 } });
});

test("无 params 字段时返回 params: null", async () => {
  readLineMock.mockResolvedValueOnce(
    '{"jsonrpc":"2.0","id":"2","method":"ping"}'
  );

  const result = await getNums();

  expect(result).toEqual({ jsonrpc: "2.0", id: "2", method: "ping", params: null });
});

test("非 JSON 行被跳过，继续读下一行", async () => {
  readLineMock
    .mockResolvedValueOnce("这不是 JSON")
    .mockResolvedValueOnce('{"jsonrpc":"2.0","id":"3","method":"echo"}');

  const result = await getNums();

  expect(result).toEqual({ jsonrpc: "2.0", id: "3", method: "echo", params: null });
  expect(readLineMock).toHaveBeenCalledTimes(2);
});

test("缺 jsonrpc/id/method 字段被跳过", async () => {
  readLineMock
    .mockResolvedValueOnce('{"foo":"bar"}')
    .mockResolvedValueOnce('{"jsonrpc":"2.0","id":"4","method":"sum","params":{"a":1,"b":2}}');

  const result = await getNums();

  expect(result).toEqual({ jsonrpc: "2.0", id: "4", method: "sum", params: { a: 1, b: 2 } });
});

test("stdin 关闭（EOF）返回 null", async () => {
  readLineMock.mockResolvedValueOnce(null);

  const result = await getNums();

  expect(result).toBeNull();
});

test("连续多次调用各自返回独立请求", async () => {
  readLineMock
    .mockResolvedValueOnce('{"jsonrpc":"2.0","id":"1","method":"add","params":{"a":1,"b":2}}')
    .mockResolvedValueOnce('{"jsonrpc":"2.0","id":"2","method":"sub","params":{"a":5,"b":3}}');

  const first = await getNums();
  const second = await getNums();

  expect(first).toEqual({ jsonrpc: "2.0", id: "1", method: "add", params: { a: 1, b: 2 } });
  expect(second).toEqual({ jsonrpc: "2.0", id: "2", method: "sub", params: { a: 5, b: 3 } });
});
