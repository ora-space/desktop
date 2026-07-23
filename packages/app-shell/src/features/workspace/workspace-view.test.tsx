import { render, waitFor } from "@testing-library/react";
import { createChatStore } from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "@ora/platform";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { WorkspaceView } from "./workspace-view";

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
});

describe("WorkspaceView", () => {
  it("reloads a selected running session after the in-memory chat store is recreated", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    state.tasks = [{ id: "t1", projectId: "p1", title: "Refresh history", status: "todo" }];
    state.sessions = [{ id: "s1", taskId: "t1", agentCli: "open_code", status: "running" }];
    const client = createMockClient(state);
    const load = vi.fn(async function* () {
      yield { type: "completed" as const };
    });
    client.session.load = load;
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(client, createTestQueryClient(), chatStore);
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await waitFor(() => expect(load).toHaveBeenCalledOnce());
    await waitFor(() => expect(chatStore.getState().conversations.s1?.isLoaded).toBe(true));
  });
});
