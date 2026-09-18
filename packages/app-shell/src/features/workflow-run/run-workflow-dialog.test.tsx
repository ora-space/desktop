import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import { createMemoryWorkflowRuntime } from "@ora/workflow-runtime/memory";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { appI18n } from "../../i18n/i18n-instance";
import { RunWorkflowDialog } from "./run-workflow-dialog";

/** Builds a dialog harness with a mocked create operation. */
function renderDialog(
  createWorkflowRun = vi.fn(async (request: { workspaceId: string }) => ({
    run: {
      id: "run-1",
      workspaceId: request.workspaceId,
      workflowId: "workflow-1",
      snapshotId: "snap-1",
      name: "Review",
      status: "pending" as const,
      state: null,
      input: null,
      output: null,
      error: null,
      startedAt: null,
      finishedAt: null,
      createdAt: 1n,
      updatedAt: 1n,
    },
  })),
) {
  const handlers: TestHandlers = { createWorkflowRun };
  const client = createTestClient(handlers);
  const runtime = createMemoryWorkflowRuntime();
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
    runtime,
  );
  const onOpenChange = vi.fn();
  render(
    <Wrapper>
      <RunWorkflowDialog
        open
        workflow={{ id: "workflow-1", name: "Review" }}
        target={{ projectId: "project-1", workspaceId: "workspace-1" }}
        onOpenChange={onOpenChange}
      />
    </Wrapper>,
  );
  return { createWorkflowRun, onOpenChange, runtime };
}

describe("RunWorkflowDialog", () => {
  beforeEach(async () => {
    await appI18n.changeLanguage("zh-CN");
  });

  it("submits injectLastFailure true by default", async () => {
    const user = userEvent.setup();
    const { createWorkflowRun, runtime } = renderDialog();
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "创建运行" }));
    await waitFor(() => {
      expect(createWorkflowRun).toHaveBeenCalledTimes(1);
    });
    expect(createWorkflowRun.mock.calls[0]?.[0]).toEqual({
      workspaceId: "workspace-1",
      workflowId: "workflow-1",
      name: "Review",
      locale: "zh-CN",
      injectLastFailure: true,
    });
    runtime.dispose();
  });

  it("submits injectLastFailure false after unchecking", async () => {
    const user = userEvent.setup();
    const { createWorkflowRun, runtime } = renderDialog();
    const dialog = await screen.findByRole("alertdialog");
    await user.click(
      screen.getByRole("checkbox", {
        name: "节点重跑时把上次失败原因告诉智能体",
      }),
    );
    await user.click(within(dialog).getByRole("button", { name: "创建运行" }));
    await waitFor(() => {
      expect(createWorkflowRun).toHaveBeenCalledTimes(1);
    });
    expect(createWorkflowRun.mock.calls[0]?.[0]).toEqual({
      workspaceId: "workspace-1",
      workflowId: "workflow-1",
      name: "Review",
      locale: "zh-CN",
      injectLastFailure: false,
    });
    runtime.dispose();
  });
});
