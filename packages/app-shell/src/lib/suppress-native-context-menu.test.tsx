import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useSuppressNativeContextMenu } from "./suppress-native-context-menu";

/** Renders a surface that suppresses the native context menu while mounted. */
function SuppressedSurface() {
  useSuppressNativeContextMenu();
  return <div data-testid="surface" />;
}

function dispatchContextMenu(): MouseEvent {
  const event = new MouseEvent("contextmenu", {
    bubbles: true,
    cancelable: true,
  });
  document.dispatchEvent(event);
  return event;
}

describe("useSuppressNativeContextMenu", () => {
  it("suppresses the native context menu while mounted and restores it after unmount", () => {
    const { unmount } = render(<SuppressedSurface />);

    expect(dispatchContextMenu().defaultPrevented).toBe(true);

    unmount();
    expect(dispatchContextMenu().defaultPrevented).toBe(false);
  });
});
