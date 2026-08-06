import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

// Node 25 exposes an incomplete experimental `localStorage` when no
// `--localstorage-file` is configured. Vitest can copy that object over jsdom's
// implementation, so install a deterministic Storage implementation for tests.
const storageEntries = new Map<string, string>();
const testLocalStorage: Storage = {
  get length() {
    return storageEntries.size;
  },
  clear: () => storageEntries.clear(),
  getItem: (key) => storageEntries.get(key) ?? null,
  key: (index) => [...storageEntries.keys()][index] ?? null,
  removeItem: (key) => {
    storageEntries.delete(key);
  },
  setItem: (key, value) => {
    storageEntries.set(key, String(value));
  },
};
Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: testLocalStorage,
});

// jsdom lacks matchMedia; settings theme subscription depends on it.
if (!window.matchMedia) {
  window.matchMedia = (query: string): MediaQueryList => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }) as MediaQueryList;
}

// jsdom lacks ResizeObserver. Notify consumers with the fixture's declared
// dimensions so geometry-driven libraries do not derive NaN SVG coordinates.
if (!globalThis.ResizeObserver) {
  globalThis.ResizeObserver = class {
    constructor(private readonly callback: ResizeObserverCallback) {}

    observe(target: Element) {
      const bounds = target.getBoundingClientRect();
      const width = bounds.width || (target instanceof HTMLElement ? target.clientWidth : 0);
      const height = bounds.height || (target instanceof HTMLElement ? target.clientHeight : 0);
      const contentRect = {
        x: bounds.x,
        y: bounds.y,
        top: bounds.top,
        right: bounds.left + width,
        bottom: bounds.top + height,
        left: bounds.left,
        width,
        height,
        toJSON: () => ({}),
      };
      this.callback([{
        target,
        contentRect,
        borderBoxSize: [],
        contentBoxSize: [],
        devicePixelContentBoxSize: [],
      }], this);
    }

    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}

// React Flow reads the viewport's vertical scale while processing observed
// node dimensions. jsdom does not provide DOMMatrixReadOnly.
if (!window.DOMMatrixReadOnly) {
  Object.defineProperty(window, "DOMMatrixReadOnly", {
    configurable: true,
    value: class {
      readonly m22: number;

      constructor(transform = "") {
        const scale = /scale\(([^)]+)\)/u.exec(transform)?.[1];
        this.m22 = scale === undefined ? 1 : Number(scale);
      }
    },
  });
}

// jsdom does not implement scrollIntoView; cmdk scrolls its active item into
// view whenever the selection changes.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

// jsdom does not implement the Web Animations API; Base UI's ScrollArea checks
// it after mount before recalculating scrollbar geometry.
if (!Element.prototype.getAnimations) {
  Element.prototype.getAnimations = () => [];
}

// crypto.randomUUID is used by mock-data; jsdom provides it in modern Node, but
// keep a stable fallback so tests are deterministic across environments.
if (!globalThis.crypto) {
  globalThis.crypto = {
    randomUUID: () => `test-${Math.random().toString(36).slice(2)}`,
  } as Crypto;
}
