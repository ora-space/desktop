// console 保护必须在最前面——覆盖后才暴露其他 API
import "../console-guard";

export { getNums } from "./getNums";
export { returnNums } from "./returnNums";
