import { test, expect, mock, beforeEach } from "bun:test";

// Mock writeLine — 先 mock 再 import returnNums
const writeLineMock = mock();
mock.module("../../src/internal/writer", () => ({
  writeLine: writeLineMock,
}));

const { returnNums } = await import("../../src/host/returnNums");

beforeEach(() => {
  writeLineMock.mockClear();
});

test("成功响应——序列化为 JSON-RPC 格式", async () => {
  await returnNums("1", 3);

  const output = JSON.parse(writeLineMock.mock.calls[0][0]);
  expect(output).toEqual({ jsonrpc: "2.0", id: "1", result: 3 });
});

test("undefined 结果序列化为 null", async () => {
  await returnNums("1", undefined);

  const output = JSON.parse(writeLineMock.mock.calls[0][0]);
  expect(output).toEqual({ jsonrpc: "2.0", id: "1", result: null });
});

test("复杂对象正确序列化", async () => {
  await returnNums("1", { name: "test", items: [1, 2] });

  const output = JSON.parse(writeLineMock.mock.calls[0][0]);
  expect(output).toEqual({
    jsonrpc: "2.0",
    id: "1",
    result: { name: "test", items: [1, 2] },
  });
});

test("错误响应——序列化为 JSON-RPC error 格式", async () => {
  await returnNums.error("1", -32601, "Unknown method: foo");

  const output = JSON.parse(writeLineMock.mock.calls[0][0]);
  expect(output).toEqual({
    jsonrpc: "2.0",
    id: "1",
    error: { code: -32601, message: "Unknown method: foo" },
  });
});

test("returnNums.error 和 returnNums 是同一个对象", () => {
  expect(returnNums.error).toBeDefined();
  expect(typeof returnNums.error).toBe("function");
});
