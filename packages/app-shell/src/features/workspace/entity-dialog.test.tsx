import { PlatformProvider, type PlatformAdapter } from "@ora/platform";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
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
    renderDialog({ selectPath });

    await user.click(screen.getByRole("button", { name: /Browse|浏览/ }));

    expect(selectPath).toHaveBeenCalledWith({
      kind: "directory",
      initialPath: "/home/ora/old",
    });
    expect(screen.getByLabelText("Path")).toHaveValue("/home/ora/new");
  });

  it("preserves the typed path when the selection is cancelled", async () => {
    const user = userEvent.setup();
    renderDialog({ selectPath: vi.fn().mockResolvedValue(null) });

    const pathInput = screen.getByLabelText("Path");
    await user.clear(pathInput);
    await user.type(pathInput, "/custom/path");
    await user.click(screen.getByRole("button", { name: /Browse|浏览/ }));

    expect(pathInput).toHaveValue("/custom/path");
  });
});
