import { describe, expect, it } from "vitest";
import {
  diffFileScrollTop,
  isDiffFileAligned,
  isDiffFileScrollSettled,
} from "./task-diff-scroll";

/** Builds a stub element whose geometry tests can set independently of jsdom layout. */
function geometryElement(values: {
  clientHeight?: number;
  offsetHeight?: number;
  offsetTop?: number;
  scrollTop?: number;
  top: number;
  height: number;
}): HTMLElement {
  const element = document.createElement("div");
  Object.defineProperty(element, "clientHeight", {
    configurable: true,
    value: values.clientHeight ?? values.height,
  });
  Object.defineProperty(element, "offsetHeight", {
    configurable: true,
    value: values.offsetHeight ?? values.height,
  });
  Object.defineProperty(element, "offsetTop", {
    configurable: true,
    value: values.offsetTop ?? values.top,
  });
  Object.defineProperty(element, "scrollTop", {
    configurable: true,
    value: values.scrollTop ?? 0,
  });
  element.getBoundingClientRect = () =>
    ({
      x: 0,
      y: values.top,
      top: values.top,
      left: 0,
      right: 800,
      bottom: values.top + values.height,
      width: 800,
      height: values.height,
      toJSON() {
        return {};
      },
    }) as DOMRect;
  return element;
}

describe("diffFileScrollTop", () => {
  it("returns null while the diff viewport has not received a height", () => {
    const root = geometryElement({
      clientHeight: 0,
      height: 0,
      top: 0,
    });
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 800,
      height: 120,
      top: 800,
    });

    expect(diffFileScrollTop(root, file)).toBeNull();
  });

  it("returns null while the requested file section has not laid out", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
    });
    const file = geometryElement({
      offsetHeight: 0,
      offsetTop: 0,
      height: 0,
      top: 0,
    });

    expect(diffFileScrollTop(root, file)).toBeNull();
  });

  it("returns null when a later file still reports a zero offset into the viewport", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
      scrollTop: 0,
    });
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 800,
      height: 120,
      top: 0,
    });

    expect(diffFileScrollTop(root, file)).toBeNull();
  });

  it("places a later file near the top of the viewport", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
      scrollTop: 0,
    });
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 800,
      height: 120,
      top: 800,
    });

    expect(diffFileScrollTop(root, file)).toBe(784);
  });

  it("keeps the first file at the top of the viewport", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
      scrollTop: 0,
    });
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 0,
      height: 120,
      top: 0,
    });

    expect(diffFileScrollTop(root, file)).toBe(0);
  });
});

describe("isDiffFileAligned", () => {
  it("is false while a later file still sits below the viewport top", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
    });
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 800,
      height: 120,
      top: 800,
    });

    expect(isDiffFileAligned(root, file)).toBe(false);
  });

  it("is false after a jump based on placeholder height overshoots the file", () => {
    const root = geometryElement({
      clientHeight: 727,
      height: 727,
      top: 105,
    });
    const file = geometryElement({
      offsetHeight: 544,
      offsetTop: 3525,
      height: 544,
      top: -842,
    });

    expect(isDiffFileAligned(root, file)).toBe(false);
  });

  it("is true when the requested file sits near the viewport top", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
    });
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 800,
      height: 120,
      top: 16,
    });

    expect(isDiffFileAligned(root, file)).toBe(true);
  });
});

describe("isDiffFileScrollSettled", () => {
  it("is false while a preceding file is still a virtualized placeholder", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
    });
    const previous = geometryElement({
      offsetHeight: 4488,
      offsetTop: 0,
      height: 4488,
      top: 0,
    });
    previous.innerHTML = '<div aria-busy="true"></div>';
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 4488,
      height: 120,
      top: 16,
    });
    root.append(previous, file);

    expect(isDiffFileAligned(root, file)).toBe(true);
    expect(isDiffFileScrollSettled(root, file)).toBe(false);
  });

  it("is true once preceding files have rendered and the target is aligned", () => {
    const root = geometryElement({
      clientHeight: 400,
      height: 400,
      top: 0,
    });
    const previous = geometryElement({
      offsetHeight: 3525,
      offsetTop: 0,
      height: 3525,
      top: -3510,
    });
    previous.innerHTML = '<div aria-busy="false"></div>';
    const file = geometryElement({
      offsetHeight: 120,
      offsetTop: 3525,
      height: 120,
      top: 16,
    });
    root.append(previous, file);

    expect(isDiffFileScrollSettled(root, file)).toBe(true);
  });
});
