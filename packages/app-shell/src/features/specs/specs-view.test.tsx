import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "@ora/platform";
import type { ResolveSpecSourceResponse, SpecCatalogResponse } from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { ContractsClientContext } from "../../contracts-client-context";
import { AppI18nProvider } from "../../i18n/i18n";
import { queryKeys } from "../../state/hooks/query-keys";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { createStubPlatform } from "../../test/stub-platform";
import { SpecSourceDialog } from "./spec-source-dialog";
import { invalidateSpecQueries, resolveMarkdownLink } from "./spec-query-utils";
import { SpecsContent } from "./specs-view";

/** Creates a retry-free query client so failures remain deterministic. */
function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

/** Wraps a Spec surface in its transport, query, platform, i18n, and tooltip providers. */
function renderSpecSurface(
  element: ReactNode,
  client = createMockClient(createMockClientState()),
  platform = createStubPlatform(),
) {
  const queryClient = createQueryClient();
  return {
    ...render(
      <QueryClientProvider client={queryClient}>
        <ContractsClientContext.Provider value={client}>
          <PlatformProvider adapter={platform}>
            <AppI18nProvider>
              <TooltipProvider>{element}</TooltipProvider>
            </AppI18nProvider>
          </PlatformProvider>
        </ContractsClientContext.Provider>
      </QueryClientProvider>,
    ),
    queryClient,
  };
}

describe("SpecsContent", () => {
  it("renders catalog Markdown and navigates only to catalog-member relative documents", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.spec.catalog = vi.fn(async () => ({
      sources: [{
        relativePath: "docs/specs",
        workflow: { kind: "custom", name: "Architecture" },
        origin: "default",
        visibility: "enabled",
        availability: "available",
      }],
      documents: [
        {
          relativePath: "docs/specs/design.md",
          sourceRelativePath: "docs/specs",
          workflow: { kind: "custom", name: "Architecture" },
          byteSize: 30,
        },
        {
          relativePath: "docs/specs/plan.mdx",
          sourceRelativePath: "docs/specs",
          workflow: { kind: "custom", name: "Architecture" },
          byteSize: 7,
        },
      ],
      truncated: false,
    } satisfies SpecCatalogResponse));
    client.spec.read = vi.fn(async ({ relativePath }) => ({
      relativePath,
      content: relativePath.endsWith("design.md")
        ? "# Design\n\n[Plan](./plan.mdx)\n\n<script>unsafe()</script>"
        : "# Plan\n",
      byteSize: relativePath.endsWith("design.md") ? 57 : 7,
    }));
    client.spec.watch = (_request, options) => (async function* () {
      yield* [];
      await new Promise<void>((resolve) => {
        if (options?.signal?.aborted) resolve();
        else options?.signal?.addEventListener("abort", () => resolve(), { once: true });
      });
    })();

    renderSpecSurface(
      <SpecsContent
        projectId="project-1"
        projectRootPath="C:/repo"
      />,
      client,
    );

    expect(await screen.findByText(/选择一个 Spec 文档|Select a Spec document/)).toBeInTheDocument();
    expect(client.spec.read).not.toHaveBeenCalled();

    await user.click(await screen.findByRole("button", { name: "specs" }));
    await user.click(await screen.findByRole("button", { name: "design.md" }));
    expect(await screen.findByRole("heading", { name: "Design" })).toBeInTheDocument();
    expect(document.querySelector("script")).toBeNull();
    await user.click(screen.getByRole("link", { name: "Plan" }));
    expect(await screen.findByRole("heading", { name: "Plan" })).toBeInTheDocument();
    expect(client.spec.read).toHaveBeenLastCalledWith(
      { target: { kind: "project", projectId: "project-1" }, relativePath: "docs/specs/plan.mdx" },
      expect.any(Object),
    );

    fireEvent.change(screen.getByPlaceholderText(/按文件名或路径筛选|Filter by file name or path/), {
      target: { value: "design.md" },
    });
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "plan.mdx" })).not.toBeInTheDocument();
    });
  });

  it("uses the existing directory picker and locks workflow fields for non-manual sources", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    const update = vi.fn(client.spec.updateProjectSources);
    client.spec.updateProjectSources = update;
    client.spec.resolveSource = vi.fn(async () => ({
      relativePath: "architecture",
      workflow: { kind: "custom", name: "Custom" },
    } satisfies ResolveSpecSourceResponse));
    const selectPath = vi.fn(async () => "C:/repo/architecture");
    const platform = { ...createStubPlatform(), selectPath };

    renderSpecSurface(
      <SpecSourceDialog
        open
        projectId="project-1"
        target={{ kind: "project", projectId: "project-1" }}
        initialPath="C:/repo"
        sources={[
          {
            relativePath: "docs/specs",
            workflow: { kind: "custom", name: "Custom" },
            origin: "default",
            visibility: "enabled",
            availability: "available",
          },
          {
            relativePath: "notes",
            workflow: { kind: "custom", name: "Notes" },
            origin: "manual",
            visibility: "enabled",
            availability: "available",
          },
        ]}
        onOpenChange={() => undefined}
      />,
      client,
      platform,
    );

    expect(screen.getByText(/^当前工作区$|^Current workspace$/)).toBeInTheDocument();
    expect(screen.getByTitle("C:/repo")).toBeInTheDocument();
    const workflowSelectors = screen.getAllByRole("combobox");
    expect(workflowSelectors[0]).toBeDisabled();
    expect(workflowSelectors[1]).toBeEnabled();
    await user.click(screen.getByRole("button", { name: /添加目录|Add directory/ }));
    expect(selectPath).toHaveBeenCalledWith({ kind: "directory", initialPath: "C:/repo" });
    expect(await screen.findByText("architecture")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /保存|Save/ }));
    expect(update).toHaveBeenCalledWith(
      expect.objectContaining({
        projectId: "project-1",
        sources: expect.arrayContaining([
          expect.objectContaining({ relativePath: "architecture", visibility: "enabled" }),
        ]),
      }),
    );
  });

  it("resolves a task workspace before opening its directory picker", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.task.getWorkspace = vi.fn(async () => ({
      workspace: { rootPath: "C:/repo/.ora-worktrees/task-1", branchName: "ora/task-1" },
    }));
    const selectPath = vi.fn(async () => null);

    renderSpecSurface(
      <SpecsContent
        projectId="project-1"
        projectRootPath="C:/repo"
        taskId="task-1"
      />,
      client,
      { ...createStubPlatform(), selectPath },
    );

    await waitFor(() => expect(client.task.getWorkspace).toHaveBeenCalledWith({ taskId: "task-1" }));
    await user.click(screen.getByRole("button", { name: /管理来源|Manage sources/ }));
    const addDirectory = screen.getByRole("button", { name: /添加目录|Add directory/ });
    await waitFor(() => expect(addDirectory).toBeEnabled());
    await user.click(addDirectory);
    expect(selectPath).toHaveBeenCalledWith({
      kind: "directory",
      initialPath: "C:/repo/.ora-worktrees/task-1",
    });
  });

  it("invalidates document content precisely and catalogs for structural changes", () => {
    const queryClient = createQueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockResolvedValue();

    invalidateSpecQueries(queryClient, "project-1", "task:task-1", [
      { kind: "modified", path: "docs/specs/design.md" },
    ]);
    expect(invalidate).toHaveBeenCalledOnce();
    expect(invalidate).toHaveBeenLastCalledWith({
      queryKey: queryKeys.specDocument("project-1", "task:task-1", "docs/specs/design.md"),
    });

    invalidate.mockClear();
    invalidateSpecQueries(queryClient, "project-1", "task:task-1", [
      { kind: "renamed", from: "docs/specs/old.md", path: "docs/specs/new.md" },
    ]);
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: queryKeys.specCatalog("project-1", "task:task-1"),
    });
  });

  it("normalizes safe relative Markdown links without allowing workspace escape", () => {
    expect(resolveMarkdownLink("docs/specs/design.md", "../plans/release.mdx#steps"))
      .toBe("docs/plans/release.mdx");
    expect(resolveMarkdownLink("design.md", "../outside.md")).toBeNull();
    expect(resolveMarkdownLink("docs/specs/design.md", "diagram.png")).toBeNull();
  });
});
