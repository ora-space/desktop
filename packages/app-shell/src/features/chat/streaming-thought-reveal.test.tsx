import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StreamingThoughtReveal } from "./streaming-thought-reveal";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("StreamingThoughtReveal", () => {
  it("disables thought motion when reduced motion is requested", () => {
    const originalAnimate = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "animate");
    const animate = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "animate", {
      configurable: true,
      value: animate,
    });
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: true })));

    try {
      render(<StreamingThoughtReveal>Checking files</StreamingThoughtReveal>);

      expect(animate).not.toHaveBeenCalled();
    } finally {
      if (originalAnimate === undefined) Reflect.deleteProperty(HTMLElement.prototype, "animate");
      else Object.defineProperty(HTMLElement.prototype, "animate", originalAnimate);
    }
  });
});
