import { createElement, type ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { RemoteContractError } from "@ora/contracts";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import { PlatformProvider } from "../../platform";
import { queryKeys } from "../../state/hooks/query-keys";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { createStubPlatform } from "../../test/stub-platform";
import { WorkspaceFilesView } from "./workspace-files-view";

/** Renders Files with a chat-driven path that the workspace cannot resolve. */
function renderMissingFile() {
  const client = createMockClient(createMockClientState());
  client.fileSystem.readWorkspaceFile = async () => {
    throw new RemoteContractError(
      {
        code: "file_system_path_not_found",
        params: {},
        requestId: "eb093a72-6961-4e9f-966a-3d5187958476",
      },
      null,
    );
  };
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        createElement(AppI18nProvider, null, children),
      ),
    );
  return render(
    <WorkspaceFilesView
      projectId="project-1"
      taskId="f06fdb43-1297-4ba3-9143-a7a95ee85b0b"
      hideHeader
      fileRequest={{ path: "crates/acp/src/lib.rs", requestId: 1 }}
    />,
    { wrapper },
  );
}

describe("WorkspaceFilesView missing files", () => {
  it("shows the localized missing-path copy instead of the raw transport error", async () => {
    renderMissingFile();

    expect(
      await screen.findByText(/所选路径不存在|The selected path was not found/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Remote Ora request failed/)).toBeNull();
  });
});

/** Renders Files with a readable chat-driven path and an optional line target. */
function renderRequestedFile(path: string, line?: number) {
  const client = createMockClient(createMockClientState());
  const readWorkspaceFile = vi.fn(async (request: { path: string }) => ({
    path: request.path,
    content: 'fn main() {\n    println!("hi");\n}\n',
    version: "test",
    sizeBytes: 32,
  }));
  client.fileSystem.readWorkspaceFile = readWorkspaceFile;
  client.task.getWorkspace = async () => ({
    workspace: { rootPath: "C:/repo", branchName: "task/task-1" },
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        createElement(AppI18nProvider, null, children),
      ),
    );
  const view = render(
    <WorkspaceFilesView
      projectId="project-1"
      taskId="task-1"
      hideHeader
      fileRequest={{
        path,
        requestId: 1,
        line,
        column: line === undefined ? undefined : 1,
      }}
    />,
    { wrapper },
  );
  return { ...view, readWorkspaceFile };
}

describe("WorkspaceFilesView file requests", () => {
  it("reads the requested path and passes the line to the viewer", async () => {
    const { container, readWorkspaceFile } = renderRequestedFile(
      "src/main.rs",
      2,
    );

    expect(await screen.findByText("src/main.rs:2:1")).toBeInTheDocument();
    expect(readWorkspaceFile).toHaveBeenCalledWith(
      {
        taskId: "task-1",
        path: "src/main.rs",
      },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    await waitFor(() => {
      expect(container.querySelector('[data-line-number="2"]')).toHaveAttribute(
        "aria-current",
        "location",
      );
    });
  });

  it("refetches on a new chat request so a deleted file is not shown from cache", async () => {
    const client = createMockClient(createMockClientState());
    let missing = false;
    client.fileSystem.readWorkspaceFile = async (request: { path: string }) => {
      if (missing) {
        throw new RemoteContractError(
          {
            code: "file_system_path_not_found",
            params: {},
            requestId: "eb093a72-6961-4e9f-966a-3d5187958476",
          },
          null,
        );
      }
      return {
        path: request.path,
        content: "fn main() {}\n",
        version: "test",
        sizeBytes: 12,
      };
    };
    client.task.getWorkspace = async () => ({
      workspace: { rootPath: "C:/repo", branchName: "task/task-1" },
    });
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 0 } },
    });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(AppI18nProvider, null, children),
        ),
      );
    const { rerender } = render(
      <WorkspaceFilesView
        projectId="project-1"
        taskId="task-1"
        hideHeader
        fileRequest={{ path: "src/main.rs", requestId: 1 }}
      />,
      { wrapper },
    );
    expect(await screen.findByText("src/main.rs")).toBeInTheDocument();
    expect(document.body.textContent).toMatch(/fn\s*main/);

    missing = true;
    rerender(
      <WorkspaceFilesView
        projectId="project-1"
        taskId="task-1"
        hideHeader
        fileRequest={{ path: "src/main.rs", requestId: 2 }}
      />,
    );
    expect(
      await screen.findByText(/所选路径不存在|The selected path was not found/),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/fn\s*main/);
  });
});

describe("WorkspaceFilesView project scope", () => {
  it("reads from the project checkout when no task is selected", async () => {
    const client = createMockClient(createMockClientState());
    const readProjectFile = vi.fn(async (request: { path: string }) => ({
      path: request.path,
      content: "# Project\n",
      version: "test",
      sizeBytes: 10,
    }));
    client.fileSystem.readProjectFile = readProjectFile;
    client.project.list = async () => ({
      projects: [{ id: "project-1", name: "Ora" }],
    });
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 0 } },
    });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(AppI18nProvider, null, children),
        ),
      );

    render(
      <WorkspaceFilesView
        projectId="project-1"
        hideHeader
        fileRequest={{ path: "README.md", requestId: 1 }}
      />,
      { wrapper },
    );

    expect(await screen.findByText("README.md")).toBeInTheDocument();
    expect(readProjectFile).toHaveBeenCalledWith(
      { projectId: "project-1", path: "README.md" },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  it("opens a project watch stream when no task is selected", async () => {
    const client = createMockClient(createMockClientState());
    const watchProject = vi.fn(() =>
      (async function* () {
        yield* [];
      })(),
    );
    client.fileSystem.watchProject = watchProject;
    client.project.list = async () => ({
      projects: [{ id: "project-1", name: "Ora" }],
    });
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 0 } },
    });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(AppI18nProvider, null, children),
        ),
      );

    render(<WorkspaceFilesView projectId="project-1" hideHeader />, {
      wrapper,
    });

    await waitFor(() => {
      expect(watchProject).toHaveBeenCalledWith(
        { projectId: "project-1" },
        expect.objectContaining({ signal: expect.any(AbortSignal) }),
      );
    });
  });

  it("waits for the project root before stripping an absolute file request", async () => {
    const client = createMockClient(createMockClientState());
    let resolveWorkspaces!: (value: {
      workspaces: Array<{
        id: string;
        projectId: string;
        kind: "main";
        lifecycle: "active";
      }>;
    }) => void;
    const workspacesPromise = new Promise<{
      workspaces: Array<{
        id: string;
        projectId: string;
        kind: "main";
        lifecycle: "active";
      }>;
    }>((resolve) => {
      resolveWorkspaces = resolve;
    });
    const readProjectFile = vi.fn(async (request: { path: string }) => ({
      path: request.path,
      content: "fn main() {}\n",
      version: "test",
      sizeBytes: 12,
    }));
    client.fileSystem.readProjectFile = readProjectFile;
    client.workspace.list = () => workspacesPromise;
    const platform = {
      ...createStubPlatform(),
      locationActions: {
        ...createStubPlatform().locationActions,
        resolveWorkspaceCwd: async () => "C:/repo",
      },
    };
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 0 } },
    });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(
            AppI18nProvider,
            null,
            createElement(PlatformProvider, { adapter: platform, children }),
          ),
        ),
      );

    render(
      <WorkspaceFilesView
        projectId="project-1"
        hideHeader
        fileRequest={{ path: "C:/repo/src/main.rs", requestId: 1 }}
      />,
      { wrapper },
    );

    expect(readProjectFile).not.toHaveBeenCalled();
    resolveWorkspaces({
      workspaces: [
        {
          id: "workspace-1",
          projectId: "project-1",
          kind: "main",
          lifecycle: "active",
        },
      ],
    });
    expect(await screen.findByText("src/main.rs")).toBeInTheDocument();
    expect(readProjectFile).toHaveBeenCalledWith(
      { projectId: "project-1", path: "src/main.rs" },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  it("defers an absolute file request when the project list query errors", async () => {
    const client = createMockClient(createMockClientState());
    const readProjectFile = vi.fn(async (request: { path: string }) => ({
      path: request.path,
      content: "fn main() {}\n",
      version: "test",
      sizeBytes: 12,
    }));
    client.fileSystem.readProjectFile = readProjectFile;
    client.workspace.list = async () => {
      throw new RemoteContractError(
        {
          code: "internal_error",
          params: {},
          requestId: "eb093a72-6961-4e9f-966a-3d5187958476",
        },
        null,
      );
    };
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 0 } },
    });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(AppI18nProvider, null, children),
        ),
      );

    render(
      <WorkspaceFilesView
        projectId="project-1"
        hideHeader
        fileRequest={{ path: "C:/repo/src/main.rs", requestId: 1 }}
      />,
      { wrapper },
    );

    // A failed project list never yields a checkout root, so the absolute path
    // must stay deferred rather than flow unstripped to readProjectFile.
    await waitFor(() => {
      expect(queryClient.getQueryState(queryKeys.workspaces)?.status).toBe(
        "error",
      );
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    expect(readProjectFile).not.toHaveBeenCalled();
  });
});
