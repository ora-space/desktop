import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { TaskGitActions } from "./task-git-actions";

/** Renders the Git action popover with deterministic callbacks for interaction tests. */
function renderGitActions(message = "") {
  const callbacks = {
    onOpenChange: vi.fn(),
    onMessageChange: vi.fn(),
    onCommit: vi.fn(),
    onCommitAndPush: vi.fn(),
    onPush: vi.fn(),
  };

  render(
    <AppI18nProvider>
      <TaskGitActions
        open
        message={message}
        additions={12}
        deletions={3}
        pending={false}
        {...callbacks}
      />
    </AppI18nProvider>,
  );

  return callbacks;
}

describe("task Git actions", () => {
  it("shows commit actions and requires a message before committing", async () => {
    const user = userEvent.setup();
    const callbacks = renderGitActions();

    expect(screen.getByRole("textbox", { name: "提交说明" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "提交" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "提交并推送" })).toBeDisabled();
    expect(screen.getByText("包含所有未提交的更改")).toBeInTheDocument();
    expect(screen.getByText("+12")).toBeInTheDocument();
    expect(screen.getByText("-3")).toBeInTheDocument();

    await user.type(screen.getByRole("textbox", { name: "提交说明" }), "fix diff layout");
    expect(callbacks.onMessageChange).toHaveBeenCalled();
  });

  it("routes the combined and push actions to their callbacks", async () => {
    const user = userEvent.setup();
    const callbacks = renderGitActions("fix diff layout");

    expect(screen.getByRole("button", { name: "提交" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "提交并推送" }));
    await user.click(screen.getByRole("button", { name: "推送" }));

    expect(callbacks.onCommitAndPush).toHaveBeenCalledOnce();
    expect(callbacks.onPush).toHaveBeenCalledOnce();
  });
});
