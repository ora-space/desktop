import { PlatformProvider, type PlatformAdapter } from "../../platform";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { EntityDialog, type EntityField } from "./entity-dialog";

const fields: EntityField[] = [
  { kind: "text", name: "name", label: "Name", value: "Ora" },
  {
    kind: "path",
    name: "rootPath",
    label: "Path",
    value: "/home/ora/old",
    selectionKind: "directory",
  },
];

/** Renders the form under the same explicit platform and locale providers as AppShell. */
function renderDialog(platform: PlatformAdapter) {
  render(
    <AppI18nProvider>
      <PlatformProvider adapter={platform}>
        <EntityDialog
          open
          title="Project"
          description="Choose a project"
          submitLabel="Save"
          fields={fields}
          onOpenChange={() => {}}
          onSubmit={async () => {}}
        />
      </PlatformProvider>
    </AppI18nProvider>,
  );
}

describe("EntityDialog path field", () => {
  it("passes the current path as a directory initial path and fills the selection", async () => {
    const user = userEvent.setup();
    const selectPath = vi.fn().mockResolvedValue("/home/ora/new");
    renderDialog({
      ...createStubPlatform(),
      selectPath,
    });

    await user.click(screen.getByRole("button", { name: /Browse|浏览/ }));

    expect(selectPath).toHaveBeenCalledWith({
      kind: "directory",
      initialPath: "/home/ora/old",
    });
    expect(screen.getByLabelText("Path")).toHaveValue("/home/ora/new");
  });

  it("preserves the typed path when the selection is cancelled", async () => {
    const user = userEvent.setup();
    renderDialog({
      ...createStubPlatform(),
      selectPath: vi.fn().mockResolvedValue(null),
    });

    const pathInput = screen.getByLabelText("Path");
    await user.clear(pathInput);
    await user.type(pathInput, "/custom/path");
    await user.click(screen.getByRole("button", { name: /Browse|浏览/ }));

    expect(pathInput).toHaveValue("/custom/path");
  });
});

/** Renders a submit-focused dialog under the same providers as AppShell. */
function renderSubmitDialog(params: {
  onSubmit: (values: Record<string, string>) => Promise<void>;
  pendingLabel?: string;
  fields?: EntityField[];
}) {
  render(
    <AppI18nProvider>
      <PlatformProvider adapter={createStubPlatform()}>
        <EntityDialog
          open
          title="Project"
          submitLabel="Save"
          pendingLabel={params.pendingLabel}
          fields={
            params.fields ?? [
              { kind: "text", name: "name", label: "Name", value: "Ora" },
            ]
          }
          onOpenChange={() => {}}
          onSubmit={params.onSubmit}
        />
      </PlatformProvider>
    </AppI18nProvider>,
  );
}

describe("EntityDialog submit loading", () => {
  it("shows a spinner on the submit button while the request is in flight", async () => {
    const user = userEvent.setup();
    let releaseSubmit: () => void = () => {};
    const pending = new Promise<void>((resolve) => {
      releaseSubmit = resolve;
    });
    renderSubmitDialog({
      pendingLabel: "Creating...",
      onSubmit: () => pending,
    });

    await user.click(screen.getByRole("button", { name: "Save" }));

    const submitButton = screen.getByRole("button", { name: "Creating..." });
    expect(submitButton).toBeDisabled();
    expect(submitButton).toHaveAttribute("aria-busy", "true");
    expect(submitButton.querySelector("[data-slot=spinner]")).not.toBeNull();
    expect(screen.getByLabelText("Name")).toBeDisabled();

    releaseSubmit();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    });
  });

  it("ignores a second submit while the first request is in flight", async () => {
    let releaseSubmit: () => void = () => {};
    const pending = new Promise<void>((resolve) => {
      releaseSubmit = resolve;
    });
    const onSubmit = vi.fn(() => pending);
    renderSubmitDialog({ onSubmit });

    const form = screen.getByRole("dialog").querySelector("form");
    expect(form).not.toBeNull();
    fireEvent.submit(form!);
    fireEvent.submit(form!);

    expect(onSubmit).toHaveBeenCalledTimes(1);
    releaseSubmit();
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
  });

  it("allows a retry after the in-flight submit fails", async () => {
    const user = userEvent.setup();
    const onSubmit = vi
      .fn()
      .mockRejectedValueOnce(new Error("unavailable"))
      .mockResolvedValueOnce(undefined);
    renderSubmitDialog({ onSubmit });

    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(2));
  });

  it("disables submit while a select field is still loading", () => {
    renderSubmitDialog({
      onSubmit: async () => {},
      fields: [
        {
          kind: "select",
          name: "baseBranch",
          label: "Base branch",
          value: "",
          options: [],
          loading: true,
        },
      ],
    });

    const submitButton = screen.getByRole("button", { name: "Save" });
    expect(submitButton).toBeDisabled();
    expect(submitButton).toHaveAttribute("aria-busy", "true");
    expect(submitButton.querySelector("[data-slot=spinner]")).not.toBeNull();
    expect(
      screen.getByRole("combobox", { name: "Base branch" }),
    ).toHaveAttribute("aria-busy", "true");
  });

  it("still shows required-field feedback when a non-loading field is empty", () => {
    const onSubmit = vi.fn(async () => {});
    renderSubmitDialog({
      onSubmit,
      fields: [
        { kind: "text", name: "title", label: "Title", value: "" },
        {
          kind: "select",
          name: "baseBranch",
          label: "Base branch",
          value: "",
          options: [],
          loading: true,
        },
      ],
    });

    const form = screen.getByRole("dialog").querySelector("form");
    expect(form).not.toBeNull();
    fireEvent.submit(form!);

    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      /Complete all required fields|请填写所有必填字段/,
    );
  });

  it("explains a blocked Enter submit when a required select is still loading", () => {
    const onSubmit = vi.fn(async () => {});
    renderSubmitDialog({
      onSubmit,
      fields: [
        { kind: "text", name: "title", label: "Title", value: "Task" },
        {
          kind: "select",
          name: "baseBranch",
          label: "Base branch",
          value: "",
          options: [],
          loading: true,
        },
      ],
    });

    const form = screen.getByRole("dialog").querySelector("form");
    expect(form).not.toBeNull();
    fireEvent.submit(form!);

    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      /Options are still loading|选项仍在加载/,
    );
  });

  it("explains a blocked Enter submit when required values are present but options are still loading", () => {
    const onSubmit = vi.fn(async () => {});
    renderSubmitDialog({
      onSubmit,
      fields: [
        { kind: "text", name: "title", label: "Title", value: "Task" },
        {
          kind: "select",
          name: "baseBranch",
          label: "Base branch",
          value: "main",
          options: [{ label: "main", value: "main" }],
          loading: true,
        },
      ],
    });

    const form = screen.getByRole("dialog").querySelector("form");
    expect(form).not.toBeNull();
    fireEvent.submit(form!);

    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      /Options are still loading|选项仍在加载/,
    );
  });
});

describe("EntityDialog searchable select", () => {
  const longName = `feature/${"a".repeat(80)}`;
  const branchFields: EntityField[] = [
    {
      kind: "searchable-select",
      name: "baseBranch",
      label: "Base branch",
      value: "refs/heads/main",
      options: [
        { label: "main", value: "refs/heads/main", keywords: "main" },
        {
          label: "213124",
          value: "refs/heads/ora/6f9888a9",
          keywords: "ora/6f9888a9",
        },
        { label: longName, value: "refs/heads/long", keywords: longName },
      ],
    },
  ];

  async function openBranchMenu() {
    const user = userEvent.setup();
    const search = screen.getByRole("combobox", { name: "Base branch" });
    await user.click(search);
    return { user, search };
  }

  it("wraps a long branch name inside the menu", async () => {
    renderSubmitDialog({ onSubmit: async () => {}, fields: branchFields });
    await openBranchMenu();

    const option = await screen.findByRole("option", { name: longName });
    expect(option.textContent).toBe(longName);
    expect(option.className).not.toContain("last-child]:hidden");
    expect(option.querySelector("span")?.className).toContain(
      "wrap-break-word",
    );
    expect(option.querySelector("span")?.className).toContain(
      "whitespace-normal",
    );
  });

  it("filters by the git branch name behind a task title", async () => {
    renderSubmitDialog({ onSubmit: async () => {}, fields: branchFields });
    const { user, search } = await openBranchMenu();
    await user.clear(search);
    await user.type(search, "6f9888a9");

    expect(await screen.findByRole("option", { name: "213124" })).toBeTruthy();
    expect(screen.queryByRole("option", { name: "main" })).toBeNull();
    expect(screen.queryByRole("option", { name: longName })).toBeNull();
  });

  it("shows an empty state when nothing matches", async () => {
    renderSubmitDialog({ onSubmit: async () => {}, fields: branchFields });
    const { user, search } = await openBranchMenu();
    await user.clear(search);
    await user.type(search, "missing-branch");

    expect(
      screen.getByText(/No matching branches|没有匹配的分支/),
    ).toBeTruthy();
    expect(screen.queryByRole("option", { name: "main" })).toBeNull();
  });

  it("submits the ref of the branch chosen from search", async () => {
    const onSubmit = vi.fn(async () => {});
    renderSubmitDialog({ onSubmit, fields: branchFields });
    const { user, search } = await openBranchMenu();
    await user.clear(search);
    await user.type(search, "6f9888a9");
    await user.click(await screen.findByRole("option", { name: "213124" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith({
        baseBranch: "refs/heads/ora/6f9888a9",
      }),
    );
  });

  it("wraps a Chinese name and an unbroken string inside the menu", async () => {
    const chinese =
      "测试/这是一条用来检查基础分支下拉框的超长中文分支名看文字会不会被裁掉";
    const unbroken = "a".repeat(80);
    renderSubmitDialog({
      onSubmit: async () => {},
      fields: [
        {
          kind: "searchable-select",
          name: "baseBranch",
          label: "Base branch",
          value: "refs/heads/main",
          options: [
            { label: "main", value: "refs/heads/main", keywords: "main" },
            { label: chinese, value: "refs/heads/zh", keywords: chinese },
            {
              label: unbroken,
              value: "refs/heads/unbroken",
              keywords: unbroken,
            },
          ],
        },
      ],
    });
    await openBranchMenu();

    const menu = document.querySelector("[data-slot=combobox-content]");
    expect(menu?.className).toContain("max-w-(--anchor-width)");
    for (const name of [chinese, unbroken]) {
      const option = await screen.findByRole("option", { name });
      const label = option.querySelector("span");
      expect(option.textContent).toBe(name);
      expect(label?.className).toContain("wrap-break-word");
      expect(label?.className).toContain("whitespace-normal");
      expect(label?.className).not.toMatch(/truncate|line-clamp|text-ellipsis/);
    }
  });

  it("submits the ref of the last branch in a list longer than 20", async () => {
    const options = Array.from({ length: 21 }, (_, index) => {
      const name = `branch-${String(index + 1).padStart(2, "0")}`;
      return {
        label: name,
        value: `refs/heads/${name}`,
        keywords: name,
      };
    });
    const last = options[20]!;
    const onSubmit = vi.fn(async () => {});
    renderSubmitDialog({
      onSubmit,
      fields: [
        {
          kind: "searchable-select",
          name: "baseBranch",
          label: "Base branch",
          value: options[0]!.value,
          options,
        },
      ],
    });
    const { user } = await openBranchMenu();

    const list = document.querySelector("[data-slot=combobox-list]");
    expect(list?.className).toContain("max-h-52");
    expect(list?.className).toContain("overflow-y-auto");
    expect(screen.getAllByRole("option")).toHaveLength(21);
    await user.click(screen.getByRole("option", { name: last.label }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith({ baseBranch: last.value }),
    );
  });

  it("shows the full branch name when the selected text overflows", async () => {
    const user = userEvent.setup();
    renderSubmitDialog({
      onSubmit: async () => {},
      fields: [{ ...branchFields[0]!, value: "refs/heads/long" }],
    });
    const input = screen.getByRole("combobox", { name: "Base branch" });
    Object.defineProperty(input, "scrollWidth", {
      configurable: true,
      value: 400,
    });
    Object.defineProperty(input, "clientWidth", {
      configurable: true,
      value: 80,
    });

    await user.hover(input);

    expect(await screen.findByText(longName)).toBeTruthy();
  });

  it("leaves a fitting branch name without a preview", async () => {
    const user = userEvent.setup();
    renderSubmitDialog({ onSubmit: async () => {}, fields: branchFields });
    const input = screen.getByRole("combobox", { name: "Base branch" });
    Object.defineProperty(input, "scrollWidth", {
      configurable: true,
      value: 40,
    });
    Object.defineProperty(input, "clientWidth", {
      configurable: true,
      value: 80,
    });

    await user.hover(input);
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 400));
    });

    expect(screen.queryByRole("dialog", { name: "main" })).toBeNull();
    expect(document.querySelector("[data-slot=popover-content]")).toBeNull();
  });
});
