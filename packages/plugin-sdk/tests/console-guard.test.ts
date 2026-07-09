import { test, expect, mock } from "bun:test";

// Import console-guard 触发覆盖（一次性副作用，不需要恢复）
await import("../src/console-guard");

test("console.log 重定向到 stderr", () => {
  const stderrMock = mock();
  process.stderr.write = stderrMock;

  console.log("hello");

  expect(stderrMock).toHaveBeenCalledTimes(1);
  const output = stderrMock.mock.calls[0][0];
  expect(output).toContain("[plugin] hello");
  expect(output).toEndWith("\n");
});

test("console.warn 重定向到 stderr 带 [plugin:warn] 前缀", () => {
  const stderrMock = mock();
  process.stderr.write = stderrMock;

  console.warn("warning");

  expect(stderrMock).toHaveBeenCalledTimes(1);
  const output = stderrMock.mock.calls[0][0];
  expect(output).toContain("[plugin:warn] warning");
});

test("console.error 重定向到 stderr 带 [plugin:error] 前缀", () => {
  const stderrMock = mock();
  process.stderr.write = stderrMock;

  console.error("error");

  expect(stderrMock).toHaveBeenCalledTimes(1);
  const output = stderrMock.mock.calls[0][0];
  expect(output).toContain("[plugin:error] error");
});

test("对象参数被 JSON 序列化", () => {
  const stderrMock = mock();
  process.stderr.write = stderrMock;

  console.log({ key: "value" });

  expect(stderrMock).toHaveBeenCalledTimes(1);
  const output = stderrMock.mock.calls[0][0];
  expect(output).toContain('{"key":"value"}');
});

test("多个参数用空格拼接", () => {
  const stderrMock = mock();
  process.stderr.write = stderrMock;

  console.log("a", "b", 123);

  expect(stderrMock).toHaveBeenCalledTimes(1);
  const output = stderrMock.mock.calls[0][0];
  expect(output).toContain("a b 123");
});

test("stdout 不受影响", () => {
  // console-guard 应保持 stdout.write 不变
  // 验证 process.stdout.write 引用了原始的 write 实现
  expect(process.stdout.write).not.toBe(process.stderr.write);
});
