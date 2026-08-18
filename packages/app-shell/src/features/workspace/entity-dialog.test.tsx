import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
