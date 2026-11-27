import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  PlatformProvider,
  type DesktopUpdateCapability,
  type DesktopUpdateStatus,
  type PlatformAdapter,
} from "../../platform";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { DesktopUpdateControl } from "./desktop-update-control";

/** Builds an update capability that reports one status and records installation/check attempts. */
function createUpdates(status: DesktopUpdateStatus) {
  const install = vi.fn().mockResolvedValue(undefined);
  const check = vi.fn().mockResolvedValue(undefined);
  const listeners: ((next: DesktopUpdateStatus) => void)[] = [];
  const updates: DesktopUpdateCapability = {
    getStatus: async () => status,
    install,
    check,
    onStatus: async (listener) => {
      listeners.push(listener);
      return () => {
        listeners.splice(listeners.indexOf(listener), 1);
      };
    },
  };
  return { updates, install, check, listeners };
}

/** Renders the control under the same explicit platform and locale providers as AppShell. */
function renderControl(
  updates?: DesktopUpdateCapability,
  onReview: () => void = () => undefined,
) {
  const platform: PlatformAdapter = { ...createStubPlatform(), updates };
  render(
    <AppI18nProvider>
      <PlatformProvider adapter={platform}>
        <DesktopUpdateControl onReview={onReview} />
      </PlatformProvider>
    </AppI18nProvider>,
  );
}

describe("DesktopUpdateControl", () => {
  it("stamps the running version on a host without the update capability", () => {
    renderControl(undefined);

    expect(screen.getByText("Ora 0.0.0")).not.toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it.each<DesktopUpdateStatus>([
    { kind: "ready", version: "0.3.0", notes: [] },
    {
      kind: "manual_update",
      version: "0.3.0",
      reason: "system_package",
      notes: [],
    },
  ])(
    "leads to the offered release without installing it: $kind",
    async (status) => {
      const user = userEvent.setup();
      const { updates, install } = createUpdates(status);
      const onReview = vi.fn();
      renderControl(updates, onReview);

      const trigger = await screen.findByRole("button", { name: /0\.3\.0/ });
      expect(screen.getByTestId("desktop-update-badge")).not.toBeNull();
      await user.click(trigger);

      expect(onReview).toHaveBeenCalledTimes(1);
      expect(install).not.toHaveBeenCalled();
    },
  );

  it("reports download progress without advertising an installable release", async () => {
    const { updates } = createUpdates({
      kind: "downloading",
      version: "0.3.0",
      downloaded: 2 * 1024 * 1024,
      total: 8 * 1024 * 1024,
    });
    renderControl(updates);

    await screen.findByRole("button", {
      name: "正在下载 Ora 0.3.0（2.0 MB / 8.0 MB）",
    });
    expect(screen.queryByTestId("desktop-update-badge")).toBeNull();
  });

  it("shows a failed check and leads to its reason without advertising a release", async () => {
    const user = userEvent.setup();
    const { updates } = createUpdates({
      kind: "failed",
      operation: "check",
      message: "endpoint unreachable",
    });
    const onReview = vi.fn();
    renderControl(updates, onReview);

    const trigger = await screen.findByRole("button", {
      name: "检查更新失败：endpoint unreachable",
    });
    expect(screen.getByRole("status").textContent).toBe("检查更新失败");
    expect(screen.queryByTestId("desktop-update-badge")).toBeNull();
    await user.click(trigger);
    expect(onReview).toHaveBeenCalledTimes(1);
  });

  it("does not call a failed installation a failed check", async () => {
    const { updates } = createUpdates({
      kind: "failed",
      operation: "install",
      version: "0.3.0",
      message: "cached update digest changed",
    });
    renderControl(updates);

    const trigger = await screen.findByRole("button", {
      name: "安装 Ora 0.3.0 失败：cached update digest changed",
    });
    expect(screen.getByRole("status").textContent).toBe("安装 Ora 0.3.0 失败");
    expect(trigger.getAttribute("disabled")).toBeNull();
    expect(screen.queryByTestId("desktop-update-badge")).toBeNull();
  });

  it("triggers a manual check when the refresh button is clicked", async () => {
    const user = userEvent.setup();
    const { updates, check } = createUpdates({ kind: "current" });
    renderControl(updates);

    await user.click(await screen.findByRole("button", { name: "检查更新" }));

    expect(check).toHaveBeenCalledTimes(1);
  });

  it("disables the manual check button while a check is in flight", async () => {
    const { updates } = createUpdates({ kind: "checking" });
    renderControl(updates);

    const trigger = await screen.findByRole("button", { name: "检查更新" });
    expect(trigger.getAttribute("disabled")).not.toBeNull();
  });

  it("follows the status events published by the host", async () => {
    const { updates, listeners } = createUpdates({ kind: "current" });
    renderControl(updates);

    await screen.findByRole("button", { name: "Ora 0.0.0" });
    await waitFor(() => expect(listeners.length).toBe(1));

    act(() => {
      for (const listener of listeners) {
        listener({ kind: "ready", version: "0.4.0", notes: [] });
      }
    });

    await screen.findByRole("button", {
      name: "发现新版本 0.4.0，点击查看",
    });
    expect(screen.getByTestId("desktop-update-badge")).not.toBeNull();
  });
});
