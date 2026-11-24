import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  PlatformProvider,
  type DesktopUpdateCapability,
  type DesktopUpdateStatus,
} from "../../platform";
import { AppI18nProvider } from "../../i18n/i18n";
import type { ReleaseNote } from "../../lib/release-note";
import { createStubPlatform } from "../../test/stub-platform";
import { PendingUpdate } from "./pending-update";

const MARKUP = "<b>标记</b> [链接](https://example.invalid)";
const NOTES: ReleaseNote[] = [
  {
    version: "0.3.0",
    date: "2026-12-10",
    entries: [{ category: "feature", "zh-CN": MARKUP, "en-US": MARKUP }],
  },
];

/** Renders the pane on a host whose updater reports one status. */
function renderPending(status: DesktopUpdateStatus) {
  const install = vi.fn().mockResolvedValue(undefined);
  const getStatus = vi.fn(async () => status);
  let publish: (next: DesktopUpdateStatus) => void = () => undefined;
  const updates: DesktopUpdateCapability = {
    getStatus,
    install,
    check: async () => undefined,
    onStatus: async (listener) => {
      publish = listener;
      return () => undefined;
    },
  };
  const { container } = render(
    <AppI18nProvider>
      <PlatformProvider adapter={{ ...createStubPlatform(), updates }}>
        <PendingUpdate />
      </PlatformProvider>
    </AppI18nProvider>,
  );
  return {
    container,
    getStatus,
    install,
    publish: (next: DesktopUpdateStatus) => act(() => publish(next)),
  };
}

describe("PendingUpdate", () => {
  it("shows the notes as plain text before the user installs", async () => {
    const user = userEvent.setup();
    const { container, install } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });

    const region = await screen.findByRole("region", {
      name: "可更新到 Ora 0.3.0",
    });
    expect(within(region).getByText(MARKUP)).not.toBeNull();
    expect(container.querySelector("a, b, img")).toBeNull();
    expect(install).not.toHaveBeenCalled();

    await user.click(within(region).getByRole("button", { name: "立即更新" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("自动重启");
    expect(dialog.textContent).not.toContain(MARKUP);
    expect(install).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));
    expect(install).toHaveBeenCalledTimes(1);
  });

  it("installs nothing when the confirmation is cancelled", async () => {
    const user = userEvent.setup();
    const { install } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });

    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "取消" }));

    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(install).not.toHaveBeenCalled();
  });

  it("asks again for a release that replaced the confirmed one", async () => {
    const user = userEvent.setup();
    const { install, publish } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    await screen.findByRole("alertdialog");

    publish({
      kind: "downloading",
      version: "0.4.0",
      downloaded: 0,
      total: null,
    });
    publish({ kind: "ready", version: "0.4.0", notes: [] });

    await screen.findByRole("region", { name: "可更新到 Ora 0.4.0" });
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(install).not.toHaveBeenCalled();
  });

  it("closes the confirmation when the release changes in one step", async () => {
    const user = userEvent.setup();
    const { publish } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    await screen.findByRole("alertdialog");

    publish({ kind: "ready", version: "0.4.0", notes: [] });

    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  });

  it("still offers the installation without notes", async () => {
    renderPending({ kind: "ready", version: "0.3.0", notes: [] });

    await screen.findByRole("button", { name: "立即更新" });
    expect(screen.queryByRole("listitem")).toBeNull();
  });

  it("explains a manual update without an install action", async () => {
    renderPending({
      kind: "manual_update",
      version: "0.3.0",
      reason: "system_package",
      notes: NOTES,
    });

    await screen.findByText(/请用软件包管理器更新/);
    expect(screen.getByText(MARKUP)).not.toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("stays absent while there is nothing to review", async () => {
    const { container, getStatus } = renderPending({ kind: "current" });

    await waitFor(() => expect(getStatus).toHaveBeenCalled());
    expect(container.textContent).toBe("");
  });
});
