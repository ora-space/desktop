import { render, screen } from "@testing-library/react";
import { PlatformProvider, type PlatformAdapter } from "@ora/platform";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { TaskChangesLayout } from "./task-changes-layout";

/** Creates a desktop platform adapter without invoking real window commands. */
function desktopPlatform(): PlatformAdapter {
  return {
    ...createStubPlatform(),
    windowControls: {
      kind: "overlay",
      os: "windows",
      minimize: vi.fn(),
      toggleMaximize: vi.fn(),
      close: vi.fn(),
      isMaximized: vi.fn().mockResolvedValue(false),
      subscribeMaximized: vi.fn().mockReturnValue(() => undefined),
    },
  };
}

describe("TaskChangesLayout", () => {
  it("reserves the desktop caption-button area for the Changes trigger", () => {
    render(
      <PlatformProvider adapter={desktopPlatform()}>
        <AppI18nProvider>
          <TaskChangesLayout taskId="task-1">
            <main>Workspace</main>
          </TaskChangesLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(screen.getByRole("group", { name: /变更|Changes/ })).toHaveClass("right-28");
  });
});
