import { type ReactNode } from "react";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import type { ChatToolCall, ChatTurn, ChatTurnItem } from "@ora/chat";
import { createChatStore } from "@ora/chat";
import { AppI18nProvider } from "../../i18n/i18n";
import { ChatStoreContext } from "../../chat-store-context";
import { ContractsClientContext } from "../../contracts-client-context";
import { PlatformProvider } from "../../platform";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { createStubPlatform } from "../../test/stub-platform";
import { TaskChangesNavigationProvider } from "../diff/task-changes-navigation";
import { ChatView } from "./chat-view";

const PROJECT_ROOT = "D:/project/desktop";

/** Lets `resolveTaskCwd` and workspace queries settle before assertions. */
async function flushDesktopCwd() {
  await act(async () => {
    await Promise.resolve();
  });
}

/** Renders ChatView the way the workspace pane does: navigation + platform + task cwd. */
function renderChatView(
  turns: ChatTurn[],
  options: {
    openWorkspaceFile?: (path: string, line?: number, column?: number) => void;
    workspaceRoot?: string;
  } = {},
) {
  const openWorkspaceFile = options.openWorkspaceFile ?? vi.fn();
  const client = createMockClient(createMockClientState());
  client.task.getWorkspace = vi.fn(async () => ({
    workspace: {
      rootPath: options.workspaceRoot ?? PROJECT_ROOT,
      branchName: "main",
    },
  }));
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  const chatStore = createChatStore(client.session);
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <ChatStoreContext.Provider value={chatStore}>
          <PlatformProvider adapter={createStubPlatform()}>
            <AppI18nProvider>
              <TaskChangesNavigationProvider
                onOpenDiff={vi.fn()}
                onOpenWorkspaceFile={openWorkspaceFile}
              >
                {children}
              </TaskChangesNavigationProvider>
            </AppI18nProvider>
          </PlatformProvider>
        </ChatStoreContext.Provider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>
  );
  return {
    ...render(
      <ChatView
        taskId="task-1"
        turns={turns}
        userName="Ada"
        isResponding={false}
        error={null}
        onSend={() => {}}
      />,
      { wrapper },
    ),
    openWorkspaceFile,
  };
}

/** One completed search/glob tool whose ACP locations are only the search root. */
function globMdTool(dump: string): ChatToolCall {
  return {
    kind: "toolCall",
    id: "glob-md",
    title: "**/*.md",
    toolKind: "search",
    status: "completed",
    content: [
      {
        type: "content",
        content: { type: "text", text: dump },
      },
    ],
    locations: [{ path: "D:\\project\\desktop" }],
    createdAt: 10,
    updatedAt: 20,
  };
}

/** One ChatView turn: user prompt, glob tool, then the assistant's listed files. */
function listedFilesTurn(markdown: string, dump: string): ChatTurn {
  const items: ChatTurnItem[] = [
    globMdTool(dump),
    {
      kind: "message",
      id: "turn-1-assistant",
      role: "assistant",
      content: markdown,
      createdAt: 2,
    },
  ];
  return {
    id: "turn-1",
    userMessage: {
      kind: "message",
      id: "turn-1-user",
      role: "user",
      content: "列出所有 md 文件",
      createdAt: 1,
    },
    items,
    status: "completed",
    stopReason: null,
    error: null,
    createdAt: 1,
  };
}

describe("ChatView project-root glob file links", () => {
  it("links listed markdown files including root README.md without expanding the glob tool", async () => {
    const user = userEvent.setup();
    const dump = [
      "Found 3 files",
      "**/*.md",
      "D:\\project\\desktop\\README.md",
      "D:\\project\\desktop\\docs\\desktop-runtime.md",
      "D:\\project\\desktop\\crates\\logging\\README.md",
    ].join("\n");
    const { openWorkspaceFile } = renderChatView([
      listedFilesTurn(
        [
          "项目里的 Markdown 文件：",
          "",
          "- README.md",
          "- docs/desktop-runtime.md",
          "- crates/logging/README.md",
          "",
          "先跑 cargo test。",
        ].join("\n"),
        dump,
      ),
    ]);
    await flushDesktopCwd();

    expect(screen.queryByRole("button", { name: /cargo test/ })).toBeNull();
    expect(
      screen.queryByTestId("chat-tool-path-output"),
    ).not.toBeInTheDocument();

    await user.click(
      await screen.findByRole("button", {
        name: /打开文件 README\.md|Open file README\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "README.md",
      undefined,
      undefined,
    );
    expect(openWorkspaceFile).not.toHaveBeenCalledWith(
      "crates/logging/README.md",
      undefined,
      undefined,
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开文件 docs\/desktop-runtime\.md|Open file docs\/desktop-runtime\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/desktop-runtime.md",
      undefined,
      undefined,
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开文件 crates\/logging\/README\.md|Open file crates\/logging\/README\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "crates/logging/README.md",
      undefined,
      undefined,
    );
  });
});
