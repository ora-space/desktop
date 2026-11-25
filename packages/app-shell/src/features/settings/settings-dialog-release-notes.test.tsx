import { createChatStore } from "@ora/chat";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import type { ReleaseNote } from "../../lib/release-note";
import { PlatformProvider, type DesktopUpdateStatus } from "../../platform";
import { useUiStore } from "../../state/stores/ui-store";
import { createTestClient } from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { SettingsDialog } from "./settings-dialog";

const RECORD_KEY = "ora.releaseNotes.promptedVersion";

/** Stands in for the notes and version that the Desktop build injects, before the modules load. */
vi.hoisted(() => {
  const note = (version: string) => ({
    version,
    date: "2026-12-10",
    entries: [
      { category: "fix", "zh-CN": `已装 ${version}`, "en-US": version },
    ],
  });
  Object.assign(globalThis, {
    __ORA_APP_VERSION__: "0.7.0",
    __ORA_RELEASE_NOTES__: [note("0.7.0"), note("0.6.0")],
  });
});

const OFFERED: ReleaseNote = {
  version: "0.8.0",
  date: "2027-01-12",
  entries: [{ category: "feature", "zh-CN": "可更新内容", "en-US": "Offered" }],
};

/** Renders the real dialog on a host whose updater offers one release. */
function renderDialog(status: DesktopUpdateStatus) {
  const install = vi.fn().mockResolvedValue(undefined);
  const client = createTestClient({});
  const AppProviders = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  const platform = {
    ...createStubPlatform(),
    updates: {
      getStatus: async () => status,
      install,
      check: async () => undefined,
      onStatus: async () => () => undefined,
    },
  };
  render(<SettingsDialog />, {
    wrapper: ({ children }: { children: ReactNode }) => (
      <PlatformProvider adapter={platform}>
        <AppProviders>{children}</AppProviders>
      </PlatformProvider>
    ),
  });
  return { install };
}

describe("SettingsDialog release notes", () => {
  beforeEach(async () => {
    window.localStorage.clear();
    window.localStorage.setItem(RECORD_KEY, "0.7.0");
    await appI18n.changeLanguage("zh-CN");
    useUiStore.setState({
      settingsOpen: true,
      settingsCategory: "appearance",
    });
  });

  it.each<DesktopUpdateStatus>([
    { kind: "ready", version: "0.8.0", notes: [OFFERED] },
    {
      kind: "manual_update",
      version: "0.8.0",
      reason: "system_package",
      notes: [OFFERED],
    },
  ])(
    "opens the offered release above a history that does not list it: $kind",
    async (status) => {
      const user = userEvent.setup();
      const { install } = renderDialog(status);

      await user.click(
        await screen.findByRole("button", { name: /发现新版本 0\.8\.0/ }),
      );

      const offered = await screen.findByRole("region", {
        name: "可更新到 Ora 0.8.0",
      });
      expect(within(offered).getByText("可更新内容")).not.toBeNull();
      const installed = screen.getByRole("region", { name: "0.7.0" });
      expect(
        offered.compareDocumentPosition(installed) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).not.toBe(0);
      expect(offered.contains(installed)).toBe(false);
      // The offered version appears once, inside its own block.
      expect(
        screen
          .getAllByRole("heading", { level: 3 })
          .map((heading) => heading.textContent),
      ).toEqual(["可更新到 Ora 0.8.0", "0.8.0", "0.7.0", "0.6.0"]);
      expect(offered.contains(screen.getByText("可更新内容"))).toBe(true);
      expect(window.localStorage.getItem(RECORD_KEY)).toBe("0.7.0");
      expect(install).not.toHaveBeenCalled();
    },
  );
});
