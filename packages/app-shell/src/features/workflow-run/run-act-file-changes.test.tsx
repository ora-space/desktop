import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { WorkflowNodeFileChange } from "@ora/workflow-runtime";
import { AppI18nProvider } from "../../i18n/i18n";
import { TaskChangesNavigationProvider } from "../diff/task-changes-navigation";
import { RunActFileChanges } from "./run-act-file-changes";

const files: WorkflowNodeFileChange[] = [
  {
    path: "/data/worktrees/run-1/src/foo.ts",
    additions: 2,
    deletions: 1,
  },
];

describe("RunActFileChanges", () => {
  it("opens the worktree-relative path when a file is clicked", async () => {
    const openDiff = vi.fn();
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <TaskChangesNavigationProvider
          onOpenDiff={openDiff}
          onOpenWorkspaceFile={vi.fn()}
        >
          <RunActFileChanges files={files} />
        </TaskChangesNavigationProvider>
      </AppI18nProvider>,
    );

    // The payload path is absolute under the managed worktree; the task Changes
    // panel matches on the worktree-relative path, so the click must open the
    // normalized form.
    await user.click(screen.getByRole("button", { name: /src\/foo\.ts/ }));
    expect(openDiff).toHaveBeenCalledWith("src/foo.ts");
  });
});
