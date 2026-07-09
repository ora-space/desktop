// 此文件在 import 时自动执行，不需要手动调用

const originalStdoutWrite = process.stdout.write.bind(process.stdout);

function format(args: any[]): string {
  return args
    .map((a) => (typeof a === "object" ? JSON.stringify(a) : String(a)))
    .join(" ");
}

console.log = (...args: any[]) => {
  process.stderr.write(`[plugin] ${format(args)}\n`);
};
console.warn = (...args: any[]) => {
  process.stderr.write(`[plugin:warn] ${format(args)}\n`);
};
console.error = (...args: any[]) => {
  process.stderr.write(`[plugin:error] ${format(args)}\n`);
};
