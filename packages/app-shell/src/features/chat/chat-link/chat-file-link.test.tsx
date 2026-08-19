import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider, type PlatformAdapter } from "../../../platform";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../../i18n/i18n";
import { createStubPlatform } from "../../../test/stub-platform";
import { TaskChangesNavigationProvider } from "../../diff/task-changes-navigation";
import type { SessionArtifactIndex } from "./artifact-index";
import { ChatFileLink } from "./chat-file-link";
import { ChatLinkContext } from "./context";

const index: SessionArtifactIndex = {
  edited: ["src/main.rs"],
  referenced: ["src/lib.rs"],
};

function desktopPlatform(open = vi.fn()): PlatformAdapter {
  return {
    ...createStubPlatform(),
    locationActions: {
      resolveTaskCwd: async () => "C:/repo",
      open,
    },
  };
}

/** Lets `resolveTaskCwd` settle so CI's stderr-as-failure gate stays quiet. */
async function flushDesktopCwd() {
  await act(async () => {
    await Promise.resolve();
  });
}

async function renderFileLink(
  raw: string,
  options?: {
    platform?: PlatformAdapter;
    openDiff?: (path: string, line?: number) => void;
    openWorkspaceFile?: (path: string, line?: number, column?: number) => void;
  },
) {
  const openDiff = options?.openDiff ?? vi.fn();
  const openWorkspaceFile = options?.openWorkspaceFile ?? vi.fn();
  const view = render(
    <PlatformProvider adapter={options?.platform ?? createStubPlatform()}>
      <AppI18nProvider>
        <TaskChangesNavigationProvider
          onOpenDiff={openDiff}
          onOpenWorkspaceFile={openWorkspaceFile}
        >
          <ChatLinkContext.Provider value={{ index, taskId: "task-1" }}>
            <ChatFileLink source="inline-code" raw={raw}>
              {raw}
            </ChatFileLink>
          </ChatLinkContext.Provider>
        </TaskChangesNavigationProvider>
      </AppI18nProvider>
    </PlatformProvider>,
  );
  await flushDesktopCwd();
  return { ...view, openDiff, openWorkspaceFile };
}

describe("ChatFileLink", () => {
  it("opens edited files in Changes and referenced files in Files", async () => {
    const user = userEvent.setup();
    const edited = await renderFileLink("src/main.rs");
    await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
    expect(edited.openDiff).toHaveBeenCalledWith("src/main.rs", undefined);

    edited.unmount();
    const referenced = await renderFileLink("src/lib.rs");
    await user.click(screen.getByRole("button", { name: /src\/lib\.rs/ }));
    expect(referenced.openWorkspaceFile).toHaveBeenCalledWith(
      "src/lib.rs",
      undefined,
      undefined,
    );
  });

  it("passes a parsed line to openDiff", async () => {
    const user = userEvent.setup();
    const { openDiff } = await renderFileLink("src/main.rs:12");
    await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
    expect(openDiff).toHaveBeenCalledWith("src/main.rs", 12);
  });

  it("keeps commands as plain code", async () => {
    await renderFileLink("cargo test");
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("cargo test").tagName).toBe("CODE");
    expect(screen.getByText("cargo test")).toHaveClass("bg-muted/80");
  });

  it("styles a clickable path as a Codex file citation, not a code chip", async () => {
    await renderFileLink("src/main.rs");
    const button = screen.getByRole("button", { name: /src\/main\.rs/ });
    expect(button).toHaveClass("text-sky-700");
    expect(button.className).toContain("decoration-dashed");
    expect(button.className).toContain("hover:underline");
    expect(button.className).not.toContain("bg-muted/80");
    expect(button.querySelector("code")).toHaveClass("text-inherit");
  });

  it("styles a Markdown file href with the same citation chrome", async () => {
    const openWorkspaceFile = vi.fn();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={openWorkspaceFile}
          >
            <ChatLinkContext.Provider value={{ index, taskId: "task-1" }}>
              <ChatFileLink source="href" raw="docs/guide.md">
                guide
              </ChatFileLink>
            </ChatLinkContext.Provider>
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    await flushDesktopCwd();
    const button = screen.getByRole("button", { name: /docs\/guide\.md/ });
    expect(button).toHaveClass("text-sky-700");
    expect(button.className).toContain("decoration-dashed");
    expect(button.className).toContain("hover:underline");
  });

  it("offers Explorer and VS Code without Terminal on the desktop host", async () => {
    await renderFileLink("src/main.rs", { platform: desktopPlatform() });
    fireEvent.contextMenu(
      screen.getByRole("button", { name: /src\/main\.rs/ }),
    );
    expect(
      screen.getByRole("menuitem", { name: /文件管理器|Explorer/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: /VS Code/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("menuitem", { name: /终端|Terminal/ }),
    ).toBeNull();
    expect(
      screen.getByRole("menuitem", { name: /复制路径|Copy path/ }),
    ).toBeInTheDocument();
  });

  it("hides OS path actions until an absolute cwd is known", async () => {
    const platform: PlatformAdapter = {
      ...createStubPlatform(),
      locationActions: {
        resolveTaskCwd: () => new Promise(() => undefined),
        open: vi.fn(),
      },
    };
    await renderFileLink("src/main.rs", { platform });
    fireEvent.contextMenu(
      screen.getByRole("button", { name: /src\/main\.rs/ }),
    );
    expect(
      screen.queryByRole("menuitem", { name: /文件管理器|Explorer/ }),
    ).toBeNull();
    expect(screen.queryByRole("menuitem", { name: /VS Code/ })).toBeNull();
    expect(
      screen.queryByRole("menuitem", { name: /复制路径|Copy path/ }),
    ).toBeNull();
    expect(
      screen.getByRole("menuitem", { name: /在文件中预览|Preview in Files/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("menuitem", { name: /在变更中查看|View in Changes/ }),
    ).toBeNull();
  });

  it("opens Explorer with the OS-absolute path from resolveTaskCwd", async () => {
    const user = userEvent.setup();
    const open = vi.fn();
    const platform = desktopPlatform(open);
    await renderFileLink("src/main.rs", { platform });
    fireEvent.contextMenu(
      screen.getByRole("button", { name: /src\/main\.rs/ }),
    );
    expect(
      screen.getByRole("menuitem", { name: /文件管理器|Explorer/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: /VS Code/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("menuitem", { name: /终端|Terminal/ }),
    ).toBeNull();
    expect(
      screen.getByRole("menuitem", { name: /在文件中预览|Preview in Files/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("menuitem", { name: /在变更中查看|View in Changes/ }),
    ).toBeNull();

    await user.click(
      screen.getByRole("menuitem", { name: /文件管理器|Explorer/ }),
    );
    await waitFor(() => {
      expect(open).toHaveBeenCalledWith("explorer", "C:/repo/src/main.rs");
    });
  });

  it("does not offer an in-app alternate on a read-only Files link", async () => {
    await renderFileLink("src/lib.rs", { platform: desktopPlatform() });
    fireEvent.contextMenu(screen.getByRole("button", { name: /src\/lib\.rs/ }));
    expect(
      screen.getByRole("menuitem", { name: /文件管理器|Explorer/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("menuitem", { name: /在文件中预览|Preview in Files/ }),
    ).toBeNull();
    expect(
      screen.queryByRole("menuitem", { name: /在变更中查看|View in Changes/ }),
    ).toBeNull();
  });
});
