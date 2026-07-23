import type { ContractsClient, ListDirectoryResponse } from "@ora/contracts";
import { PlatformHost, PlatformProvider } from "../index";
import { PathSelectionInProgressError, type SelectPathOptions } from "../types";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { createWebPlatformAdapter, type WebPlatformAdapter } from "./web-platform-adapter";

const homeDirectory: ListDirectoryResponse = {
  currentPath: "/home/ora",
  parentPath: "/home",
  breadcrumbs: [
    { name: "/", path: "/" },
    { name: "home", path: "/home" },
    { name: "ora", path: "/home/ora" },
  ],
  entries: [
    {
      name: ".hidden",
      path: "/home/ora/.hidden",
      kind: "directory",
      isSymbolicLink: false,
    },
    {
      name: "projects",
      path: "/home/ora/projects",
      kind: "directory",
      isSymbolicLink: false,
    },
    {
      name: "notes.txt",
      path: "/home/ora/notes.txt",
      kind: "file",
      isSymbolicLink: false,
    },
  ],
};

/** Builds the narrow contracts client surface owned by the Web adapter. */
function fileSystemClient(listDirectory = vi.fn().mockResolvedValue(homeDirectory)) {
  return {
    client: { fileSystem: { listDirectory } } as unknown as ContractsClient,
    listDirectory,
  };
}

/** Mounts the injected adapter and exposes a real focusable caller for restoration assertions. */
function PickerHarness({
  adapter,
  options = { kind: "directory" },
}: {
  adapter: WebPlatformAdapter;
  options?: SelectPathOptions;
}) {
  const [result, setResult] = useState("pending");
  return (
    <PlatformProvider adapter={adapter}>
      <button
        type="button"
        onClick={() => void adapter.selectPath(options).then((path) => setResult(path ?? "cancelled"))}
      >
        Browse
      </button>
      <output>{result}</output>
      <PlatformHost locale="en-US" />
    </PlatformProvider>
  );
}

describe("WebPlatformAdapter", () => {
  it("rejects a second selection while preserving the first request", async () => {
    const { client } = fileSystemClient();
    const adapter = createWebPlatformAdapter(client);
    const firstSelection = adapter.selectPath({ kind: "directory" });

    await expect(adapter.selectPath({ kind: "file" })).rejects.toBeInstanceOf(
      PathSelectionInProgressError,
    );
    adapter.completeSelection(1, "/home/ora");
    await expect(firstSelection).resolves.toBe("/home/ora");
  });

  it("lists home, displays hidden entries, and returns the selected directory", async () => {
    const user = userEvent.setup();
    const { client, listDirectory } = fileSystemClient();
    const adapter = createWebPlatformAdapter(client);
    render(<PickerHarness adapter={adapter} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));
    expect(await screen.findByText(".hidden")).toBeVisible();
    expect(listDirectory).toHaveBeenCalledWith({});

    await user.click(screen.getByText("projects"));
    await user.click(screen.getByRole("button", { name: "Select" }));

    expect(await screen.findByText("/home/ora/projects")).toBeVisible();
    expect(screen.getByRole("button", { name: "Browse" })).toHaveFocus();
  });

  it("uses a stable desktop workspace while remaining viewport-bound", async () => {
    const user = userEvent.setup();
    const { client } = fileSystemClient();
    const adapter = createWebPlatformAdapter(client);
    render(<PickerHarness adapter={adapter} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveStyle({
      display: "flex",
      flexDirection: "column",
      inlineSize: "40rem",
      blockSize: "30rem",
      maxInlineSize: "calc(100vw - 2rem)",
      maxBlockSize: "calc(100dvh - 2rem)",
    });
    expect(screen.getByRole("listbox")).toHaveClass("min-h-0", "flex-1", "overflow-auto");
  });

  it("uses the same single-column list for a small directory", async () => {
    const user = userEvent.setup();
    const { client } = fileSystemClient();
    const adapter = createWebPlatformAdapter(client);
    render(<PickerHarness adapter={adapter} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));

    const entry = await screen.findByRole("option", { name: "projects" });
    expect(entry).toHaveClass("absolute", "w-full");
    expect(entry.parentElement).toHaveClass("relative", "w-full");
  });

  it("summarizes a deep path instead of rendering every breadcrumb", async () => {
    const currentPath = "/home/ora/projects/very-long-organization-name/very-long-repository-name";
    const directory: ListDirectoryResponse = {
      ...homeDirectory,
      currentPath,
      breadcrumbs: [
        ...homeDirectory.breadcrumbs,
        { name: "projects", path: "/home/ora/projects" },
        { name: "very-long-organization-name", path: "/home/ora/projects/very-long-organization-name" },
        { name: "very-long-repository-name", path: currentPath },
      ],
    };
    const user = userEvent.setup();
    const { client } = fileSystemClient(vi.fn().mockResolvedValue(directory));
    const adapter = createWebPlatformAdapter(client);
    render(<PickerHarness adapter={adapter} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));

    expect(await screen.findByRole("textbox", { name: "Absolute path" })).toHaveValue(currentPath);
    expect(screen.queryByRole("button", { name: "projects" })).not.toBeInTheDocument();
  });

  it("returns files in file mode while keeping directories navigable", async () => {
    const user = userEvent.setup();
    const { client } = fileSystemClient();
    const adapter = createWebPlatformAdapter(client);
    render(<PickerHarness adapter={adapter} options={{ kind: "file" }} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));
    await user.click(await screen.findByText("notes.txt"));
    await user.click(screen.getByRole("button", { name: "Select" }));

    expect(await screen.findByText("/home/ora/notes.txt")).toBeVisible();
  });

  it("supports keyboard selection in file mode", async () => {
    const user = userEvent.setup();
    const { client } = fileSystemClient();
    const adapter = createWebPlatformAdapter(client);
    render(<PickerHarness adapter={adapter} options={{ kind: "file" }} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));
    await screen.findByText("notes.txt");
    const list = screen.getByRole("listbox");
    list.focus();
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{Enter}");

    expect(await screen.findByText("/home/ora/notes.txt")).toBeVisible();
  });

  it("retries the failed destination and cannot submit stale selections while it is unavailable", async () => {
    const projectsDirectory: ListDirectoryResponse = {
      currentPath: "/home/ora/projects",
      parentPath: "/home/ora",
      breadcrumbs: [...homeDirectory.breadcrumbs, { name: "projects", path: "/home/ora/projects" }],
      entries: [],
    };
    const listDirectory = vi
      .fn()
      .mockResolvedValueOnce(homeDirectory)
      .mockRejectedValueOnce(new Error("unreadable"))
      .mockResolvedValueOnce(projectsDirectory);
    const user = userEvent.setup();
    const { client } = fileSystemClient(listDirectory);
    const adapter = createWebPlatformAdapter(client);
    render(<PickerHarness adapter={adapter} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));
    await user.dblClick(await screen.findByText("projects"));

    expect(await screen.findByRole("alert")).toBeVisible();
    expect(screen.getByRole("button", { name: "Select current folder" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Select" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("This folder is empty")).toBeVisible();
    expect(listDirectory).toHaveBeenNthCalledWith(2, { path: "/home/ora/projects" });
    expect(listDirectory).toHaveBeenNthCalledWith(3, { path: "/home/ora/projects" });
  });

  it("renders a new virtual range after scrolling a large directory", async () => {
    const largeDirectory: ListDirectoryResponse = {
      ...homeDirectory,
      entries: Array.from({ length: 100 }, (_, index) => ({
        name: `file-${index}.txt`,
        path: `/home/ora/file-${index}.txt`,
        kind: "file" as const,
        isSymbolicLink: false,
      })),
    };
    const { client } = fileSystemClient(vi.fn().mockResolvedValue(largeDirectory));
    const adapter = createWebPlatformAdapter(client);
    const user = userEvent.setup();
    render(<PickerHarness adapter={adapter} options={{ kind: "file" }} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));
    expect(await screen.findByText("file-0.txt")).toBeVisible();
    const list = screen.getByRole("listbox");
    list.scrollTop = 3_312;
    fireEvent.scroll(list);

    await waitFor(() => expect(screen.getByText("file-99.txt")).toBeVisible());
  });

  it("resets the directory list to the top after navigating", async () => {
    const nextPath = "/home/ora/next";
    const largeDirectory: ListDirectoryResponse = {
      ...homeDirectory,
      entries: Array.from({ length: 100 }, (_, index) => ({
        name: `file-${index}.txt`,
        path: `/home/ora/file-${index}.txt`,
        kind: "file" as const,
        isSymbolicLink: false,
      })),
    };
    const nextDirectory: ListDirectoryResponse = {
      ...homeDirectory,
      currentPath: nextPath,
      entries: [],
    };
    const listDirectory = vi
      .fn()
      .mockResolvedValueOnce(largeDirectory)
      .mockResolvedValueOnce(nextDirectory);
    const { client } = fileSystemClient(listDirectory);
    const adapter = createWebPlatformAdapter(client);
    const user = userEvent.setup();
    render(<PickerHarness adapter={adapter} options={{ kind: "file" }} />);

    await user.click(screen.getByRole("button", { name: "Browse" }));
    await screen.findByText("file-0.txt");
    const list = screen.getByRole("listbox");
    list.scrollTop = 3_312;
    fireEvent.scroll(list);

    const pathInput = screen.getByRole("textbox", { name: "Absolute path" });
    await user.clear(pathInput);
    await user.type(pathInput, nextPath);
    await user.click(screen.getByRole("button", { name: "Go" }));

    expect(await screen.findByText("This folder is empty")).toBeVisible();
    expect(list.scrollTop).toBe(0);
  });

  it("falls back to home when the supplied initial path cannot be read", async () => {
    const listDirectory = vi
      .fn()
      .mockRejectedValueOnce(new Error("missing"))
      .mockResolvedValueOnce(homeDirectory);
    const { client } = fileSystemClient(listDirectory);
    const adapter = createWebPlatformAdapter(client);

    const selection = adapter.selectPath({ kind: "directory", initialPath: "/missing" });
    render(
      <PlatformProvider adapter={adapter}>
        <PlatformHost locale="en-US" />
      </PlatformProvider>,
    );

    expect(await screen.findByText("projects")).toBeVisible();
    expect(listDirectory).toHaveBeenNthCalledWith(1, { path: "/missing" });
    expect(listDirectory).toHaveBeenNthCalledWith(2, {});

    // Cancelling outside act would leave the dialog unmount cascade outside React's test scheduler.
    await act(async () => {
      adapter.completeSelection(1, null);
      await selection;
    });
  });
});
