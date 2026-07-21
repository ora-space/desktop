import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import type { Project, Session, Task } from "@ora/contracts";
import { createChatStore, type AcpClient } from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { createMockClient, createMockClientState, type MockClientState } from "../../test/mock-client";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { WorkspaceView } from "./workspace-view";

const PROJECT: Project = { id: "p1", name: "Ora", rootPath: "/ora" };
const TASK: Task = { id: "t1", projectId: "p1", title: "Refactor", status: "todo" };
/** A session the agent never opened, which is the one case a worktree cannot rescue. */
const SESSION_WITHOUT_AGENT: Session = {
  id: "s1",
  taskId: "t1",
  agentId: "codex",
  agentSessionId: null,
  status: "running",
};

interface RecordedAcp {
  newSessionRequests: Parameters<AcpClient["newSession"]>[0][];
  promptRequests: Parameters<AcpClient["prompt"]>[0][];
}

/** Builds a chat store whose ACP calls are recorded so tests can assert the whole request. */
function createRecordingChatStore(): { chatStore: ReturnType<typeof createChatStore> } & RecordedAcp {
  const newSessionRequests: RecordedAcp["newSessionRequests"] = [];
  const promptRequests: RecordedAcp["promptRequests"] = [];
  const chatStore = createChatStore({
    newSession: async (request) => {
      newSessionRequests.push(request);
      return { sessionId: "agent-session-created" };
    },
    prompt: async (request) => {
      promptRequests.push(request);
      return { stopReason: "end_turn" };
    },
    subscribe: () => () => undefined,
  });
  return { chatStore, newSessionRequests, promptRequests };
}

/**
 * Renders the pane with the same provider stack AppShell gives it. Reuses the
 * hook harness's wrapper so component tests and hook tests share one setup.
 */
function renderWorkspaceView(state: MockClientState, chatStore: ReturnType<typeof createChatStore>) {
  const Wrapper = createHookWrapper(createMockClient(state), createTestQueryClient(), chatStore);
  return render(
    <Wrapper>
      <AppI18nProvider>
        <TooltipProvider>
          <WorkspaceView userName="Eric" />
        </TooltipProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

/** Waits for the workspace queries to settle, which is what enables the composer. */
async function composerEnabled(): Promise<HTMLElement> {
  await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
  return screen.getByRole("textbox");
}

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
});

describe("WorkspaceView", () => {
  it("opens an agent session for the selected worktree and sends the first message", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    const { chatStore, newSessionRequests, promptRequests } = createRecordingChatStore();
    useWorkspaceSelectionStore.getState().selectTask(TASK.id, TASK.projectId);

    renderWorkspaceView(state, chatStore);
    const textarea = await composerEnabled();
    await user.type(textarea, "ship it{Enter}");

    await waitFor(() => expect(state.sessions).toHaveLength(1));
    expect(state.sessions).toEqual([
      {
        id: "s1",
        taskId: TASK.id,
        agentId: "codex",
        agentSessionId: "agent-session-created",
        status: "running",
      },
    ]);
    // The agent session must open against the project checkout, not the process cwd.
    expect(newSessionRequests).toEqual([{ cwd: PROJECT.rootPath, mcpServers: [] }]);
    await waitFor(() =>
      expect(promptRequests).toEqual([
        { sessionId: "agent-session-created", prompt: [{ type: "text", text: "ship it" }] },
      ]),
    );
  });

  it("selects the created session so the reply lands in its conversation", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    const { chatStore } = createRecordingChatStore();
    useWorkspaceSelectionStore.getState().selectTask(TASK.id, TASK.projectId);

    renderWorkspaceView(state, chatStore);
    const textarea = await composerEnabled();
    await user.type(textarea, "ship it{Enter}");

    await waitFor(() =>
      expect(useWorkspaceSelectionStore.getState().selection).toEqual({
        projectId: PROJECT.id,
        taskId: TASK.id,
        sessionId: "s1",
      }),
    );
    expect(await screen.findByText("ship it")).toBeInTheDocument();
  });

  it("blocks sending until a project and worktree are chosen, without an inline error", async () => {
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    const { chatStore, newSessionRequests } = createRecordingChatStore();

    renderWorkspaceView(state, chatStore);

    const textarea = await screen.findByRole("textbox");
    expect(textarea).toBeDisabled();
    // The hint moved to hover, so nothing should be shouting in the layout.
    expect(screen.queryByRole("alert")).toBeNull();
    expect(newSessionRequests).toEqual([]);
  });

  it("reports a session the agent never opened instead of starting a second one", async () => {
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION_WITHOUT_AGENT];
    const { chatStore, newSessionRequests } = createRecordingChatStore();
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION_WITHOUT_AGENT.id, TASK.id, PROJECT.id);

    renderWorkspaceView(state, chatStore);

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeDisabled();
    expect(newSessionRequests).toEqual([]);
  });
});
