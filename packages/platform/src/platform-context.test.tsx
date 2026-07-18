import { renderHook } from "@testing-library/react";
import { usePlatform } from "./use-platform";

it("rejects platform hooks outside an explicit provider", () => {
  expect(() => renderHook(usePlatform)).toThrow("usePlatform must be used within PlatformProvider");
});
