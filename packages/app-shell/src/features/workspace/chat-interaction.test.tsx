import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { PromptSessionEvent } from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { beforeEach, describe, expect, it } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { PlatformProvider } from "../../platform";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createScriptedChatSession } from "../../test/chat-session-harness";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
import {
  createSessionMemory,
  sessionHandlers,
} from "../../test/memory/sessions";
import {
  createAgentRuntimeMemory,
  agentRuntimeHandlers,
} from "../../test/memory/agent-runtime";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import { createSkillMemory, skillHandlers } from "../../test/memory/skills";
import "../../i18n/i18n-instance";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { WorkspaceView } from "./workspace-view";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createSessionMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createSkillMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workspaceHandlers(state),
    ...sessionHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...skillHandlers(state),
  };
}

/** Builds one assistant text frame in the same shape as the generated ACP client. */
function assistantText(
  text: string,
  messageId = "assistant-1",
): PromptSessionEvent {
  return {
    type: "session_update",
    update: {
      sessionUpdate: "agent_message_chunk",
      messageId,
      content: { type: "text", text },
    },
  };
}

/** Creates a promise whose completion is controlled by the test. */
function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  usePendingAgentStore.setState({ selections: {}, switches: {} });
});

describe("chat interaction MVP", () => {
  it("sends an Enter-submitted message and renders a controlled streaming reply", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Chat interaction",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];

    const secondChunk = deferred();
    const scriptedSession = createScriptedChatSession(async function* () {
      yield assistantText("第一段");
      await secondChunk.promise;
      yield assistantText("第二段");
      yield { type: "completed", stopReason: "end_turn" };
    });
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient({
      ...baseClientHandlers,
      ...scriptedSession.handlers,
    });
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");

    const view = render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Tester" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    const composer = await screen.findByRole("textbox");
    await waitFor(() => expect(composer).toBeEnabled());
    await user.click(composer);
    await user.paste("第一行普通文本\n第二行普通文本\n\n\n第四行普通文本");
    await waitFor(() =>
      expect(composer.dataset.composerText).toBe(
        "第一行普通文本\n第二行普通文本\n\n\n第四行普通文本",
      ),
    );
    await user.keyboard("{Enter}");

    await waitFor(() =>
      expect(scriptedSession.promptRequests).toEqual([
        {
          sessionId: "s1",
          prompt: [
            {
              type: "text",
              text: "第一行普通文本\n第二行普通文本\n\n\n第四行普通文本",
            },
          ],
        },
      ]),
    );
    expect(screen.getByText("第一行普通文本")).toBeInTheDocument();
    expect(screen.getByText("第四行普通文本")).toBeInTheDocument();

    const response = () => view.container.querySelector("[data-turn-response]");
    await waitFor(() => expect(response()).toHaveTextContent("第一段"));
    expect(screen.getByRole("button", { name: /停止|Stop/ })).toBeEnabled();

    secondChunk.resolve();

    await waitFor(() => expect(response()).toHaveTextContent("第一段第二段"));
    await waitFor(() => expect(composer).toBeEnabled());
    expect(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    ).toBeDisabled();
  });
});
