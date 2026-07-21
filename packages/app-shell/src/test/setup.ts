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

// crypto.randomUUID is used by runtime stores; jsdom provides it in modern Node, but
// keep a stable fallback so tests are deterministic across environments.
if (!globalThis.crypto) {
  globalThis.crypto = {
    randomUUID: () => `test-${Math.random().toString(36).slice(2)}`,
  } as Crypto;
}

