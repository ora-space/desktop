const originalWrite = process.stdout.write.bind(process.stdout);

/**
 * 向 stdout 写入一行。仅 SDK 内部调用。
 */
export function writeLine(json: string): void {
  originalWrite(json + "\n");
}
