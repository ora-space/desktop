import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "@ora/platform";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { TaskChangesLayout } from "./task-changes-layout";
import { useTaskChangesNavigation } from "./task-changes-navigation-context";

vi.mock("./task-diff-view", () => ({
  TaskDiffView: ({
    toolbar,
    fileRequest,
  }: {
    toolbar?: ReactNode;
    fileRequest?: { path: string; requestId: number };
  }) => (
    <section aria-label="Task diff">
      <header data-diff-toolbar>
        <button type="button">Commit</button>
        {toolbar}
      </header>
      <span data-testid="requested-file">{fileRequest?.path}</span>
    </section>
  ),
}));

/** Requests a file through the same context used by answer-level diff summaries. */
function OpenChangedFileButton() {
  const navigation = useTaskChangesNavigation();
  return (
    <button type="button" onClick={() => navigation?.openFile("src/main.ts")}>
      Open changed file
    </button>
  );
}

describe("TaskChangesLayout", () => {
  it("positions the closed Changes trigger at the diff-toolbar coordinates", () => {
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesLayout taskId="task-1">
            <main>Workspace</main>
          </TaskChangesLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(
      screen.getByRole("group", { name: /变更|Changes/ }).parentElement,
    ).toHaveClass("right-4", "top-2");
  });

  it("moves the Changes controls beside Commit after opening", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesLayout taskId="task-1">
            <main>Workspace</main>
          </TaskChangesLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button", { name: /变更|Changes/ }));

    const toolbar = document.querySelector("[data-diff-toolbar]");
    expect(toolbar).toContainElement(
      screen.getByRole("button", { name: "Commit" }),
    );
    expect(toolbar).toContainElement(
      screen.getByRole("group", { name: /变更|Changes/ }),
    );
    expect(
      screen.getByRole("button", {
        name: /显示或隐藏变更文件目录|toggle file tree/i,
      }),
    ).toBeInTheDocument();
  });

  it("opens the Changes panel and forwards a requested answer file", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesLayout taskId="task-1">
            <OpenChangedFileButton />
          </TaskChangesLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button", { name: "Open changed file" }));

    expect(screen.getByRole("region", { name: "Task diff" })).toBeInTheDocument();
    expect(screen.getByTestId("requested-file")).toHaveTextContent("src/main.ts");
  });
});
