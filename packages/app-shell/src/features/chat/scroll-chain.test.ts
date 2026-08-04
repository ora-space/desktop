import { describe, expect, it, vi } from "vitest";
import {
  chainWheelToScrollParent,
  findVerticalScrollParent,
} from "./scroll-chain";

describe("scroll-chain", () => {
  it("forwards leftover wheel delta to the nearest vertical scroll parent", () => {
    const parent = document.createElement("div");
    Object.defineProperty(parent, "scrollHeight", { value: 800 });
    Object.defineProperty(parent, "clientHeight", { value: 400 });
    parent.scrollTop = 0;
    parent.style.overflowY = "auto";

    const child = document.createElement("div");
    Object.defineProperty(child, "scrollHeight", { value: 500 });
    Object.defineProperty(child, "clientHeight", { value: 200 });
    child.scrollTop = 300;
    parent.append(child);
    document.body.append(parent);

    const preventDefault = vi.fn();
    chainWheelToScrollParent({ deltaY: 40, preventDefault }, child);

    expect(preventDefault).toHaveBeenCalled();
    expect(parent.scrollTop).toBe(40);
    parent.remove();
  });

  it("does not chain while the nested viewport can still scroll", () => {
    const parent = document.createElement("div");
    Object.defineProperty(parent, "scrollHeight", { value: 800 });
    Object.defineProperty(parent, "clientHeight", { value: 400 });
    parent.scrollTop = 0;
    parent.style.overflowY = "auto";

    const child = document.createElement("div");
    Object.defineProperty(child, "scrollHeight", { value: 500 });
    Object.defineProperty(child, "clientHeight", { value: 200 });
    child.scrollTop = 0;
    parent.append(child);
    document.body.append(parent);

    const preventDefault = vi.fn();
    chainWheelToScrollParent({ deltaY: 40, preventDefault }, child);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(parent.scrollTop).toBe(0);
    parent.remove();
  });

  it("finds the nearest vertical scroll parent", () => {
    const outer = document.createElement("div");
    outer.style.overflowY = "auto";
    Object.defineProperty(outer, "scrollHeight", { value: 800 });
    Object.defineProperty(outer, "clientHeight", { value: 400 });

    const inner = document.createElement("div");
    outer.append(inner);
    document.body.append(outer);

    expect(findVerticalScrollParent(inner)).toBe(outer);
    outer.remove();
  });
});
