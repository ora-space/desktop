import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RemoteContractError } from "@ora/contracts";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  PlatformProvider,
  type DesktopUpdateCapability,
  type DesktopUpdateStatus,
} from "../../platform";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
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
  const check = vi.fn().mockResolvedValue(undefined);
  const getStatus = vi.fn(async () => status);
  let publish: (next: DesktopUpdateStatus) => void = () => undefined;
  const updates: DesktopUpdateCapability = {
    getStatus,
    install,
    check,
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
    check,
    container,
    getStatus,
    install,
    publish: (next: DesktopUpdateStatus) => act(() => publish(next)),
    /** Delivers several statuses in one update, as React may render them. */
    publishTogether: (...next: DesktopUpdateStatus[]) =>
      act(() => next.forEach((status) => publish(status))),
  };
}

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
});

describe("PendingUpdate", () => {
  it("shows the offered notes in the interface language", async () => {
    await appI18n.changeLanguage("en-US");
    renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: [
        {
          version: "0.3.0",
          date: "2026-12-10",
          entries: [
            {
              category: "fix",
              "zh-CN": "修复了登录",
              "en-US": "Fixed signing in",
            },
          ],
        },
      ],
    });

    const region = await screen.findByRole("region", {
      name: "Ora 0.3.0 is available",
    });
    expect(within(region).getByText("Fixed signing in")).not.toBeNull();
    expect(within(region).queryByText("修复了登录")).toBeNull();
  });

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
    expect(install).toHaveBeenCalledExactlyOnceWith("0.3.0");
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

  it("reports a rejected installation until it is confirmed again", async () => {
    const user = userEvent.setup();
    const { install } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    const confirm = async () => {
      await user.click(await screen.findByRole("button", { name: "立即更新" }));
      const dialog = await screen.findByRole("alertdialog");
      await user.click(
        within(dialog).getByRole("button", { name: "立即更新" }),
      );
    };

    // A refusing installer reaches the shell as the host's decoded internal error.
    const payload = {
      code: "internal_error",
      params: {},
      requestId: "00000000-0000-4000-8000-000000000002",
    } as const;
    install.mockRejectedValueOnce(new RemoteContractError(payload, payload));
    await confirm();
    expect((await screen.findByRole("alert")).textContent).toContain(
      "安装 Ora 0.3.0 失败",
    );

    await confirm();
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(install).toHaveBeenCalledTimes(2);
  });

  it("cannot be confirmed again until the host answers the installation", async () => {
    const user = userEvent.setup();
    const { install } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    let refuse: (error: unknown) => void = () => undefined;
    install.mockImplementationOnce(
      () =>
        new Promise<void>((_resolve, reject) => {
          refuse = reject;
        }),
    );
    const update = await screen.findByRole("button", { name: "立即更新" });
    await user.click(update);
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));

    // The host has not left `ready` yet, for example while a check holds its lock.
    expect(update.hasAttribute("disabled")).toBe(true);

    await act(async () => refuse(new Error("rejected")));
    expect(update.hasAttribute("disabled")).toBe(false);
    expect(install).toHaveBeenCalledTimes(1);
  });

  it("reports an installation whose package had to be discarded", async () => {
    const user = userEvent.setup();
    const { check, install, publish } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    install.mockImplementationOnce(() => {
      publish({
        kind: "failed",
        operation: "install",
        version: "0.3.0",
        message: "signature is invalid",
      });
      return Promise.reject(new Error("rejected"));
    });
    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));

    const region = await screen.findByRole("region", {
      name: "安装 Ora 0.3.0 失败",
    });
    expect(
      within(region).getByText("原因：signature is invalid"),
    ).not.toBeNull();
    await user.click(within(region).getByRole("button", { name: "重试" }));
    expect(check).toHaveBeenCalledTimes(1);
    // The retry is a check, so a further failure is no longer the installation's.
    publish({ kind: "checking" });
    publish({
      kind: "failed",
      operation: "check",
      message: "connection refused",
    });
    expect(screen.getByRole("region", { name: "检查更新失败" })).not.toBeNull();
  });

  it("does not blame a later check from elsewhere on the installation", async () => {
    const user = userEvent.setup();
    const { install, publish } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    install.mockImplementationOnce(() => {
      publish({
        kind: "failed",
        operation: "install",
        version: "0.3.0",
        message: "signature is invalid",
      });
      return Promise.reject(new Error("rejected"));
    });
    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));
    await screen.findByRole("region", { name: "安装 Ora 0.3.0 失败" });

    publish({ kind: "checking" });
    publish({
      kind: "failed",
      operation: "check",
      message: "connection refused",
    });
    const region = screen.getByRole("region", { name: "检查更新失败" });
    expect(within(region).getByText("原因：connection refused")).not.toBeNull();
  });

  it("names a failure by the operation the host reports, even with no checking in between", async () => {
    const { publishTogether } = renderPending({
      kind: "failed",
      operation: "install",
      version: "0.3.0",
      message: "signature is invalid",
    });
    // Mounted after the failure: nothing local remembers an installation.
    await screen.findByRole("region", { name: "安装 Ora 0.3.0 失败" });

    // React may render a `checking` and the failure after it once; the title follows the latter.
    publishTogether(
      { kind: "checking" },
      { kind: "failed", operation: "check", message: "connection refused" },
    );
    expect(screen.getByRole("region", { name: "检查更新失败" })).not.toBeNull();
  });

  it("does not report a confirmation that a newer release replaced", async () => {
    const user = userEvent.setup();
    const { install, publish } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    const payload = {
      code: "desktop_update_release_replaced",
      params: {},
      requestId: "00000000-0000-4000-8000-000000000001",
    } as const;
    install.mockRejectedValueOnce(new RemoteContractError(payload, payload));
    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));

    await waitFor(() => expect(install).toHaveBeenCalledTimes(1));
    // Even before the replacement's own status arrives, nothing blames the installation.
    expect(screen.queryByRole("alert")).toBeNull();

    publish({ kind: "ready", version: "0.4.0", notes: NOTES });
    expect(
      screen.getByRole("region", { name: "可更新到 Ora 0.4.0" }),
    ).not.toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("does not report a confirmation that nothing is ready for any more", async () => {
    const user = userEvent.setup();
    const { container, install, publish } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    const payload = {
      code: "desktop_update_no_pending_update",
      params: {},
      requestId: "00000000-0000-4000-8000-000000000003",
    } as const;
    install.mockRejectedValueOnce(new RemoteContractError(payload, payload));
    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));

    await waitFor(() => expect(install).toHaveBeenCalledTimes(1));
    // The host's own status, not this refusal, says what happened to the release.
    expect(screen.queryByRole("alert")).toBeNull();

    publish({ kind: "current" });
    expect(container.textContent).toBe("");
  });

  it("drops a rejected installation once another release replaces it", async () => {
    const user = userEvent.setup();
    const { install, publish } = renderPending({
      kind: "ready",
      version: "0.3.0",
      notes: NOTES,
    });
    install.mockRejectedValueOnce(new Error("rejected"));
    await user.click(await screen.findByRole("button", { name: "立即更新" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));
    await screen.findByRole("alert");

    publish({ kind: "ready", version: "0.4.0", notes: NOTES });
    expect(
      screen.getByRole("region", { name: "可更新到 Ora 0.4.0" }),
    ).not.toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("explains a failed check and retries it", async () => {
    const user = userEvent.setup();
    const { check } = renderPending({
      kind: "failed",
      operation: "check",
      message: "update operation failed: connection refused",
    });

    const region = await screen.findByRole("region", { name: "检查更新失败" });
    expect(within(region).getByRole("alert").textContent).toContain(
      "connection refused",
    );
    await user.click(within(region).getByRole("button", { name: "重试" }));
    expect(check).toHaveBeenCalledTimes(1);
  });

  it("stays absent while there is nothing to review", async () => {
    const { container, getStatus } = renderPending({ kind: "current" });

    await waitFor(() => expect(getStatus).toHaveBeenCalled());
    expect(container.textContent).toBe("");
  });
});
