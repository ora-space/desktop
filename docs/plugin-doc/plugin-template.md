## @ora-space/create-plugin

JSR

```
{
  "name": "@ora-space/create-plugin",
  "exports": {
    ".": "./mod.ts",
    "./create": "./create.ts"
  }
}
```

运行交互式 Prompt，并在指定的 my-weather-plugin 目录下生成标准模板文件。

scripts/build.ts

```ts
import * as esbuild from "https://deno.land/x/esbuild@v0.20.2/mod.ts";
import { denoPlugins } from "jsr:@lucacasonato/deno-plugins";

await esbuild.build({
  plugins: [...denoPlugins()],
  entryPoints: ["./main.ts"],
  outfile: "./dist/main.js",
  bundle: true, // 打平为单文件
  format: "esm", // 输出原生 ES Module
  treeShaking: true, // 开启精确 Tree-shaking 剪枝
  minify: true, // 压缩与去除无用变量
  sourcemap: false,
});

esbuild.stop();
```

通过 Deno 直接运行这个构建脚本：`deno run -A scripts/build.ts`

脚手架，orax cli 一定要有很详细的错误说明文字
