import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { TextEditContextMenu } from "./text-edit-context-menu";

void appI18n;

const ONE_PIXEL_PNG =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

describe("TextEditContextMenu", () => {
  it("shows Cut Copy Paste and Select All, disabling cut and paste when read-only", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <TextEditContextMenu
          trigger={<div data-testid="host">body</div>}
          editable={false}
          hasSelection={false}
          onCut={() => undefined}
          onCopy={() => undefined}
          onPaste={() => undefined}
          onSelectAll={() => undefined}
        >
          visible
        </TextEditContextMenu>
      </AppI18nProvider>,
    );

    fireEvent.contextMenu(screen.getByTestId("host"));

    expect(
      await screen.findByRole("menuitem", { name: "剪切" }),
    ).toHaveAttribute("data-disabled");
    expect(screen.getByRole("menuitem", { name: "复制" })).toHaveAttribute(
      "data-disabled",
    );
    expect(screen.getByRole("menuitem", { name: "粘贴" })).toHaveAttribute(
      "data-disabled",
    );
    expect(screen.getByRole("menuitem", { name: "全选" })).not.toHaveAttribute(
      "data-disabled",
    );
    await user.click(screen.getByRole("menuitem", { name: "全选" }));
  });

  it("copies the bitmap when the right-click target is a copyable image", async () => {
    const user = userEvent.setup();
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: vi.fn().mockReturnValue(true),
    });

    render(
      <AppI18nProvider>
        <TextEditContextMenu
          trigger={<div data-testid="host" />}
          editable={false}
          hasSelection={false}
          onCut={() => undefined}
          onCopy={() => undefined}
          onPaste={() => undefined}
          onSelectAll={() => undefined}
        >
          <figure data-copyable-image>
            <img alt="shot" src={ONE_PIXEL_PNG} />
          </figure>
        </TextEditContextMenu>
      </AppI18nProvider>,
    );

    fireEvent.contextMenu(screen.getByAltText("shot"));
    const copy = await screen.findByRole("menuitem", { name: "复制" });
    await waitFor(() => expect(copy).not.toHaveAttribute("data-disabled"));
    await user.click(copy);
    await waitFor(() =>
      expect(document.execCommand).toHaveBeenCalledWith("copy"),
    );
  });

  it("writes a PNG ClipboardItem when native image copy is unavailable", async () => {
    const user = userEvent.setup();
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: vi.fn().mockReturnValue(false),
    });
    const write = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal(
      "ClipboardItem",
      class {
        constructor(public items: Record<string, Blob>) {}
      },
    );
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { write, writeText: vi.fn(), readText: vi.fn() },
    });

    render(
      <AppI18nProvider>
        <TextEditContextMenu
          trigger={<div data-testid="host" />}
          editable={false}
          hasSelection={false}
          onCut={() => undefined}
          onCopy={() => undefined}
          onPaste={() => undefined}
          onSelectAll={() => undefined}
        >
          <figure data-copyable-image>
            <img alt="shot" src={ONE_PIXEL_PNG} />
          </figure>
        </TextEditContextMenu>
      </AppI18nProvider>,
    );

    fireEvent.contextMenu(screen.getByAltText("shot"));
    const copy = await screen.findByRole("menuitem", { name: "复制" });
    await waitFor(() => expect(copy).not.toHaveAttribute("data-disabled"));
    await user.click(copy);
    await waitFor(() => expect(write).toHaveBeenCalled());
  });
});
