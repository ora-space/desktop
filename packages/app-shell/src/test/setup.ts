import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
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

// jsdom lacks ResizeObserver; cmdk (the command menu behind worktree pickers)
// observes its list on mount.
if (!globalThis.ResizeObserver) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}

// jsdom does not implement scrollIntoView; cmdk scrolls its active item into
// view whenever the selection changes.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

// crypto.randomUUID is used by mock-data; jsdom provides it in modern Node, but
// keep a stable fallback so tests are deterministic across environments.
if (!globalThis.crypto) {
  globalThis.crypto = {
    randomUUID: () => `test-${Math.random().toString(36).slice(2)}`,
  } as Crypto;
}

