import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider, type PlatformAdapter } from "@ora/platform";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { TaskChangesLayout } from "./task-changes-layout";

vi.mock("./task-diff-view", () => ({
  TaskDiffView: ({ toolbar }: { toolbar?: ReactNode }) => (
    <section aria-label="Task diff">{toolbar}</section>
  ),
}));

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

    expect(screen.getByRole("group", { name: /变更|Changes/ }).parentElement)
      .toHaveClass("right-56");
  });

  it("moves the Changes controls into the diff header after opening", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={desktopPlatform()}>
        <AppI18nProvider>
          <TaskChangesLayout taskId="task-1">
            <main>Workspace</main>
          </TaskChangesLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button", { name: /变更|Changes/ }));

    const diff = screen.getByRole("region", { name: "Task diff" });
    expect(diff).toContainElement(screen.getByRole("group", { name: /变更|Changes/ }));
    expect(screen.getByRole("group", { name: /变更|Changes/ }).parentElement)
      .toBe(diff);
  });
});
