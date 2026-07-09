const decoder = new TextDecoder();
let lineBuffer = "";

/**
 * 逐行产出 stdin 数据。模块级闭包保持单例 iterator。
 */
async function* lineIterator(): AsyncGenerator<string> {
  for await (const chunk of process.stdin) {
    lineBuffer += decoder.decode(chunk, { stream: true });
    while (true) {
      const idx = lineBuffer.indexOf("\n");
      if (idx < 0) break;
      const line = lineBuffer.slice(0, idx);
      lineBuffer = lineBuffer.slice(idx + 1);
      yield line;
    }
  }
  // stdin 关闭——产出 buffer 中剩余内容
  if (lineBuffer.length > 0) {
    yield lineBuffer;
    lineBuffer = "";
  }
}

const iterator = lineIterator();

/**
 * 从 stdin 读取下一行。返回 null 表示 stdin 已关闭。
 *
 * 使用模块级闭包保持 iterator 和 buffer 单例——
 * 多次调用共享同一个 stdin 流。
 */
export async function readLine(): Promise<string | null> {
  const result = await iterator.next();
  if (result.done) return null;
  return result.value;
}
