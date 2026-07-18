import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(cleanup);

if (!Element.prototype.scrollTo) {
  Element.prototype.scrollTo = () => {};
}

if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

if (!globalThis.ResizeObserver) {
  globalThis.ResizeObserver = class ResizeObserver {
    private readonly callback: ResizeObserverCallback;

    constructor(callback: ResizeObserverCallback) {
      this.callback = callback;
    }

    observe(target: Element) {
      const contentRect = target.getBoundingClientRect();
      this.callback(
        [{ target, contentRect } as ResizeObserverEntry],
        this as unknown as globalThis.ResizeObserver,
      );
    }
    unobserve() {}
    disconnect() {}
  };
}

Object.defineProperties(HTMLElement.prototype, {
  clientHeight: { configurable: true, get: () => 288 },
  clientWidth: { configurable: true, get: () => 640 },
});

HTMLElement.prototype.getBoundingClientRect = () =>
  ({
    width: 640,
    height: 288,
    top: 0,
    right: 640,
    bottom: 288,
    left: 0,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  }) as DOMRect;

Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
  configurable: true,
  get: () => 288,
});
Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
  configurable: true,
  get: () => 640,
});
