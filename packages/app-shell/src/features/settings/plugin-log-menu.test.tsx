import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import type { ContractsClient } from "@ora/contracts";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import {
  createSettingsMemory,
  settingsHandlers,
} from "../../test/memory/settings";
import { PluginManager } from "./plugin-manager";

// Keep this test worker responsible for initializing the instance used by useTranslation.
void appI18n;

/** Plugins plus developer-mode state, which gates the log items of every row. */
function fixture(developerMode: boolean) {
  const state = { ...createPluginMemory(), ...createSettingsMemory() };
  state.developerMode = { enabled: developerMode };
  const handlers: TestHandlers = {
    ...pluginHandlers(state),
    ...settingsHandlers(state),
  };
  const plugin = state.installedPlugins[0];
  return { state, handlers, client: createTestClient(handlers), plugin };
}

/** Renders the installed-plugin manager with an isolated query client and platform. */
function renderManager(
  client: ContractsClient,
  plugins: ReturnType<typeof fixture>["state"]["installedPlugins"],
  platform: PlatformAdapter = createStubPlatform(),
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <PlatformProvider adapter={platform}>
          <AppI18nProvider>
            <PluginManager
              plugins={plugins}
              onBack={() => undefined}
              onConfigure={() => undefined}
              onImport={() => undefined}
              importing={false}
            />
          </AppI18nProvider>
        </PlatformProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
}

/** Opens one plugin row's menu. */
async function openRowMenu(
  user: ReturnType<typeof userEvent.setup>,
  displayName: string,
) {
  await user.click(
    await screen.findByRole("button", {
      name: new RegExp(`${displayName}`),
    }),
  );
}

/** Returns the log-level submenu trigger once its level has loaded. */
async function findLogLevelTrigger() {
  const trigger = await screen.findByRole("menuitem", {
    name: /日志级别|Log level/,
  });
  await waitFor(() => expect(trigger).not.toHaveAttribute("aria-disabled"));
  return trigger;
}

/**
 * Opens the submenu and activates one level entirely by keyboard.
 *
 * Pointer interaction with a submenu depends on hover timers and the safe-polygon close logic
 * of the menu library, which a loaded test worker interleaves unpredictably; keyboard
 * navigation is deterministic.
 */
async function chooseLevel(
  user: ReturnType<typeof userEvent.setup>,
  level: "trace" | "debug" | "info" | "warn" | "error",
  setSpy: { mock: { calls: unknown[][] } },
) {
  const trigger = await findLogLevelTrigger();
  // The submenu trigger is the first item of the row menu.
  await user.keyboard("{ArrowDown}");
  await waitFor(() => expect(trigger).toHaveAttribute("data-highlighted"));
  await user.keyboard("{ArrowRight}");
  await waitFor(() => expect(trigger).toHaveAttribute("aria-expanded", "true"));
  const index = ["trace", "debug", "info", "warn", "error"].indexOf(level);
  const items = await screen.findAllByRole("menuitemradio");
  await waitFor(() => expect(items[0]).toHaveAttribute("data-highlighted"));
  for (let step = 0; step < index; step += 1) {
    await user.keyboard("{ArrowDown}");
  }
  await waitFor(() => expect(items[index]).toHaveAttribute("data-highlighted"));
  await user.keyboard("{Enter}");
  await waitFor(() => expect(setSpy.mock.calls.length).toBeGreaterThan(0));
  // Radio items keep the menu open; close it so the next interaction starts from the row.
  await user.keyboard("{Escape}");
  await waitFor(() =>
    expect(screen.queryAllByRole("menuitemradio")).toHaveLength(0),
  );
  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryAllByRole("menu")).toHaveLength(0));
}

it("hides every plugin log item until developer mode is on", async () => {
  const user = userEvent.setup({ pointerEventsCheck: 0 });
  const { client, state, plugin } = fixture(false);
  renderManager(client, state.installedPlugins);

  await openRowMenu(user, plugin.displayName);

  await screen.findByRole("menuitem", { name: /卸载|Uninstall/ });
  expect(
    screen.queryByRole("menuitem", { name: /日志级别|Log level/ }),
  ).toBeNull();
  expect(
    screen.queryByRole("menuitem", { name: /下载日志|Download log/ }),
  ).toBeNull();
});

it("shows plain level names and persists a chosen level in developer mode", async () => {
  // Submenus of the menu library sit under a pointer-events guard until hovered; the
  // check is disabled the same way the sidebar context-menu tests do it.
  const user = userEvent.setup({ pointerEventsCheck: 0 });
  const { client, state, handlers, plugin } = fixture(true);
  const setSpy = vi.spyOn(handlers, "setPluginLogLevel");
  renderManager(client, state.installedPlugins);

  await openRowMenu(user, plugin.displayName);
  const trigger = await findLogLevelTrigger();
  expect(trigger).toHaveTextContent("Info");
  expect(trigger).not.toHaveTextContent(/默认|default|推荐|recommended/i);
  await user.keyboard("{Escape}");
  await openRowMenu(user, plugin.displayName);
  await chooseLevel(user, "debug", setSpy);

  await waitFor(() =>
    expect(state.pluginLogLevels.get(plugin.id)).toBe("debug"),
  );
  expect(setSpy.mock.calls[0][0]).toEqual({
    pluginId: plugin.id,
    level: "debug",
  });
  await openRowMenu(user, plugin.displayName);
  expect(await findLogLevelTrigger()).toHaveTextContent("Debug");
});

it("keeps the previous level when the host refuses to persist the change", async () => {
  const user = userEvent.setup({ pointerEventsCheck: 0 });
  const { client, state, handlers, plugin } = fixture(true);
  state.pluginLogLevels.set(plugin.id, "warn");
  const setSpy = vi
    .spyOn(handlers, "setPluginLogLevel")
    .mockRejectedValue(new Error("disk full"));
  renderManager(client, state.installedPlugins);

  await openRowMenu(user, plugin.displayName);
  await chooseLevel(user, "trace", setSpy);

  await openRowMenu(user, plugin.displayName);
  const trigger = await findLogLevelTrigger();
  await waitFor(() => expect(trigger).toHaveTextContent("Warn"));
  expect(state.pluginLogLevels.get(plugin.id)).toBe("warn");
});

it("downloads the plugin log through the platform save flow when the host offers it", async () => {
  const user = userEvent.setup({ pointerEventsCheck: 0 });
  const { client, state, plugin } = fixture(true);
  const downloadPluginLog = vi.fn(async () => true);
  const platform: PlatformAdapter = {
    ...createStubPlatform(),
    diagnosticLogs: { downloadToday: async () => true, downloadPluginLog },
  };
  renderManager(client, state.installedPlugins, platform);

  await openRowMenu(user, plugin.displayName);
  await user.click(
    await screen.findByRole("menuitem", { name: /下载日志|Download log/ }),
  );

  await waitFor(() =>
    expect(downloadPluginLog).toHaveBeenCalledWith(
      plugin.id,
      plugin.displayName,
    ),
  );
});

it("omits the download item when the host cannot export logs", async () => {
  const user = userEvent.setup({ pointerEventsCheck: 0 });
  const { client, state, plugin } = fixture(true);
  renderManager(client, state.installedPlugins);

  await openRowMenu(user, plugin.displayName);

  await findLogLevelTrigger();
  expect(
    screen.queryByRole("menuitem", { name: /下载日志|Download log/ }),
  ).toBeNull();
});
