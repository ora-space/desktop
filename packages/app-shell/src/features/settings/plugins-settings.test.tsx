import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import type {
  ContractsClient,
  InstallOutcome,
  InstalledPlugin,
  PackInstallationStatus,
  PluginLogo,
} from "@ora/contracts";
import { toast } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import { usePluginOperationStore } from "../../state/stores/plugin-operation-store";
import { useMarketplaceSyncStore } from "../../state/stores/marketplace-sync-store";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { PluginOperationEventBridge } from "./plugin-operation-event-bridge";
import { PluginsSettings } from "./plugins-settings";
import { useUiStore } from "../../state/stores/ui-store";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return { ...createPluginMemory() };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...pluginHandlers(state),
  };
}

// Keep this test worker responsible for initializing the instance used by useTranslation.
void appI18n;

afterEach(() => {
  act(() => usePluginOperationStore.setState({ activities: {} }));
  act(() =>
    useMarketplaceSyncStore.setState({
      hostRefreshing: false,
      userSyncing: false,
    }),
  );
});

/** Renders plugin settings with isolated query, contracts-client, and platform state. */
function renderSettings(
  client: ContractsClient,
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
            <PluginOperationEventBridge />
            <PluginsSettings />
          </AppI18nProvider>
        </PlatformProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
}

/** Opens the header management menu and picks "Manage plugins" from the marketplace. */
async function openManagePlugins(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    await screen.findByRole("button", {
      name: /插件管理操作|Plugin management actions/,
    }),
  );
  await user.click(
    await screen.findByRole("menuitem", { name: /管理插件|Manage plugins/ }),
  );
}

/** The host-local asset URL the backend hands out for a validated registry icon. */
const WEATHER_LOGO: PluginLogo = {
  variant: "universal",
  url: "ora-plugin://localhost/logo/official/weather/universal.svg",
};

function clientWithWeather(logo: PluginLogo | null = null) {
  const state = createFixtureState();
  // This file exercises install/import flows in isolation from the seeded agent
  // packages, so installed-plugin assertions can count exactly the fixture under test.
  state.installedPlugins = [];
  state.availablePlugins.push({
    id: "official/weather",
    name: "weather",
    title: "Weather",
    kind: "agent",
    namespace: "official",
    sourceUrl: "https://github.com/ora-space/marketplace",
    version: "1.2.0",
    description: "Weather plugin",
    logo,
    compatibility: "compatible",
  });
  const handlers = createFixtureHandlers(state);
  return { state, handlers, client: createTestClient(handlers) };
}

/** A mock installed entry so import tests can assert the committed package shape. */
function weatherInstalled(): InstalledPlugin {
  return {
    id: "official/weather",
    namespace: "official",
    name: "weather",
    displayName: "weather",
    version: "1.2.0",
    description: "Weather plugin",
    homepage: null,
    license: null,
    kind: "agent",
    agentDisplayName: "weather",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

const HOOK_ID = "official/rtk-ai.rtk";

/** An installed Hook package whose descriptor the manager row renders. */
function hookInstalled(version = "0.1.0"): InstalledPlugin {
  return {
    id: HOOK_ID,
    namespace: "official",
    name: "rtk-ai.rtk",
    displayName: "rtk-ai.rtk",
    version,
    description: "RTK command rewrite hook",
    homepage: null,
    license: null,
    kind: "hook",
    executable: "assets/rtk.exe",
    supportedAgents: ["claude-code", "codex"],
    target: "x86_64-pc-windows-msvc",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

/** One marketplace listing for the Hook, with nothing installed yet. */
function clientWithHook(availableVersion = "0.1.0") {
  const state = createFixtureState();
  state.installedPlugins = [];
  state.availablePlugins.push({
    id: HOOK_ID,
    name: "rtk-ai.rtk",
    title: "RTK",
    kind: "hook",
    namespace: "official",
    sourceUrl: "https://github.com/ora-space/marketplace",
    version: availableVersion,
    description: "RTK command rewrite hook",
    logo: null,
    compatibility: "compatible",
  });
  const handlers = createFixtureHandlers(state);
  return { state, handlers, client: createTestClient(handlers) };
}

/** One installed Hook and nothing else, so manager assertions count exactly that package. */
function clientWithInstalledHook(version = "0.1.0") {
  const state = createFixtureState();
  state.installedPlugins = [hookInstalled(version)];
  const handlers = createFixtureHandlers(state);
  return { state, handlers, client: createTestClient(handlers) };
}

const PACK_ID = "official/ora-space.python-extension-pack";
const CORE_MEMBER_ID = "official/ora-space.python-core";
const LINT_MEMBER_ID = "official/ora-space.python-lint";

/** A package-shaped skill member used by the Pack memory adapter. */
function packMember(id: string, version = "1.0.0"): InstalledPlugin {
  return {
    id,
    namespace: "official",
    name: id.slice(id.indexOf("/") + 1),
    displayName: id.slice(id.indexOf("/") + 1),
    version,
    description: "Pack member",
    homepage: null,
    license: null,
    kind: "skill",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

/** Creates one Pack listing whose installable members are explicit test state. */
function clientWithPack() {
  const state = createFixtureState();
  state.installedPlugins = [];
  state.availablePlugins.push({
    id: PACK_ID,
    name: "ora-space.python-extension-pack",
    title: "Python Extension Pack",
    kind: "pack",
    namespace: "official",
    sourceUrl: "https://github.com/ora-space/marketplace",
    version: "1.0.0",
    description: "Python development tools",
    logo: null,
    packMembers: ["ora-space.python-core"],
    compatibility: "compatible",
  });
  state.packMemberPlugins.set(PACK_ID, [packMember(CORE_MEMBER_ID)]);
  const handlers = createFixtureHandlers(state);
  return { state, handlers, client: createTestClient(handlers) };
}

/** Seeds one visible Pack journal plus the plan presented by its uninstall dialog. */
function seedInstalledPack(
  state: FixtureState,
  members: PackInstallationStatus["members"],
) {
  state.packInstallations = [
    {
      packId: PACK_ID,
      sourceUrl: "https://github.com/ora-space/marketplace",
      members,
    },
  ];
  state.packUninstallPlans.set(PACK_ID, {
    remove: members
      .filter((member) => member.ownership === "managed_by_pack")
      .map((member) => member.memberId),
    preserve: members
      .filter((member) => member.ownership === "pre_existing")
      .map((member) => ({
        memberId: member.memberId,
        reason: "pre_existing" as const,
      })),
    alreadyMissing: [],
  });
}

/** Seeds one installed plugin and its smallest editable declaration. */
function clientWithPluginConfiguration(unavailable = false) {
  const state = createFixtureState();
  state.installedPlugins.push({
    ...weatherInstalled(),
    configuration: unavailable
      ? { state: "unavailable", errorCode: "configuration_load_failed" }
      : { state: "available", completeness: "incomplete" },
  });
  state.pluginConfigurations.set("official/weather", {
    pluginId: "official/weather",
    schemaVersion: 1,
    revision: 0n,
    declarationFingerprint: "declaration-1",
    settings: [
      {
        declaration: {
          id: "endpoint",
          title: "Endpoint",
          description: "Service URL",
          type: "string",
          required: true,
          order: null,
          default: null,
        },
        storedValue: null,
        effectiveValue: null,
        redacted: false,
        source: "absent",
        valueErrorCode: null,
      },
    ],
    summary: unavailable
      ? { state: "unavailable", errorCode: "configuration_load_failed" }
      : { state: "available", completeness: "incomplete" },
  });
  return { state, client: createTestClient(createFixtureHandlers(state)) };
}

/** The browse grid is driven entirely by the backend registry index. */
it("renders marketplace plugins from the registry index", async () => {
  const { client } = clientWithWeather();
  renderSettings(client);

  expect(await screen.findByText("Weather")).toBeInTheDocument();
  expect(screen.getByText("Weather plugin")).toBeInTheDocument();
  const installButton = screen.getByRole("button", { name: /安装|Install/ });
  expect(
    installButton.querySelector(".tabler-icon-download"),
  ).toBeInTheDocument();
  expect(installButton).toHaveClass("border-border");
  expect(
    screen.getByRole("button", {
      name: /查看 Weather 的 README|View Weather README/,
    }),
  ).toHaveClass("items-center");
});

/** Installing goes through the backend and refreshes the installed surface. */
it("adopts a deep-linked marketplace search once", async () => {
  const { client } = clientWithWeather();
  act(() =>
    useUiStore
      .getState()
      .openPluginSettings({ kind: "marketplaceSearch", query: "weather" }),
  );
  renderSettings(client);

  expect(await screen.findByText("Weather")).toBeInTheDocument();
  expect(
    screen.getByRole("textbox", { name: /搜索插件|Search plugins/ }),
  ).toHaveValue("weather");
  expect(useUiStore.getState().pluginSettingsRequest).toBeNull();
  act(() => useUiStore.setState({ settingsOpen: false }));
});

it("opens a deep-linked plugin configuration inside plugin management", async () => {
  const user = userEvent.setup();
  const state = createFixtureState();
  state.installedPlugins.push({
    ...weatherInstalled(),
    configuration: { state: "available", completeness: "incomplete" },
  });
  state.pluginConfigurations.set("official/weather", {
    pluginId: "official/weather",
    schemaVersion: 1,
    revision: 0n,
    declarationFingerprint: "declaration-1",
    settings: [
      {
        declaration: {
          id: "endpoint",
          title: "Endpoint",
          description: "Service URL",
          type: "string",
          required: true,
          order: 1n,
          default: null,
        },
        storedValue: null,
        effectiveValue: null,
        redacted: false,
        source: "absent",
        valueErrorCode: null,
      },
    ],
    summary: { state: "available", completeness: "incomplete" },
  });
  act(() =>
    useUiStore.getState().openPluginSettings({
      kind: "configure",
      pluginId: "official/weather",
      displayName: "weather",
    }),
  );
  renderSettings(createTestClient(createFixtureHandlers(state)));

  expect(await screen.findByLabelText(/Endpoint/)).toBeInTheDocument();
  expect(useUiStore.getState().pluginSettingsRequest).toBeNull();

  // Leaving the editor returns to plugin management, not the marketplace grid.
  await user.click(
    screen.getByRole("button", { name: /管理插件|Manage plugins/ }),
  );
  expect(
    await screen.findByRole("button", { name: /配置|Configure/ }),
  ).toBeInTheDocument();
  act(() => useUiStore.setState({ settingsOpen: false }));
});

it("installs a marketplace plugin through the backend", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  renderSettings(client);

  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));

  await waitFor(() => expect(state.installedPlugins).toHaveLength(1));
  expect(state.installedPlugins[0]).toMatchObject({
    id: "official/weather",
    namespace: "official",
    name: "weather",
    displayName: "weather",
    version: "1.2.0",
  });
  const installedButton = await screen.findByRole("button", {
    name: /已安装|Installed/,
  });
  expect(installedButton).not.toHaveClass("border-border");
  const completed = installedButton.querySelector(
    '[data-slot="plugin-install-complete"]',
  );
  expect(completed).toHaveClass("size-6");
  expect(completed).toHaveAttribute("data-animated", "true");
  expect(completed?.querySelector(".tabler-icon-check")).toHaveClass(
    "zoom-in-0",
  );
});

/** A fresh Pack install refreshes its journal projection without faking an installed Pack. */
it("shows a fresh Pack installation immediately without creating an InstalledPlugin", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithPack();
  renderSettings(client);

  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));

  expect(
    await screen.findByRole("heading", {
      name: /已安装集合包|Installed packs/,
    }),
  ).toBeInTheDocument();
  expect(state.packInstallations).toHaveLength(1);
  expect(state.installedPlugins.map((plugin) => plugin.id)).toEqual([
    CORE_MEMBER_ID,
  ]);
  expect(state.installedPlugins.some((plugin) => plugin.id === PACK_ID)).toBe(
    false,
  );
});

/** Reinstalling a Pack refreshes the projection after filling a newly declared member. */
it("refreshes Pack installations after a reinstall fills a missing member", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithPack();
  renderSettings(client);
  const install = await screen.findByRole("button", { name: /安装|Install/ });
  await user.click(install);
  await screen.findByRole("heading", { name: /已安装集合包|Installed packs/ });

  state.packMemberPlugins.get(PACK_ID)?.push(packMember(LINT_MEMBER_ID));
  await user.click(install);

  await waitFor(() =>
    expect(state.packInstallations[0]?.members).toHaveLength(2),
  );
  expect(await screen.findByText(LINT_MEMBER_ID)).toBeInTheDocument();
});

/** A failed Pack member is operation failure, even though the transport returned an outcome. */
it("reports a Pack member failure with an error toast", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithPack();
  state.installOutcome = {
    state: "pack_installed",
    members: [],
    skipped: [],
    failed: {
      pluginId: CORE_MEMBER_ID,
      errorCode: "plugin_download_failed",
      rollbackFailures: [],
    },
  };
  const successToast = vi
    .spyOn(toast, "success")
    .mockImplementation(() => "toast");
  const errorToast = vi
    .spyOn(toast, "error")
    .mockClear()
    .mockImplementation(() => "toast");
  renderSettings(client);

  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));

  await waitFor(() => expect(errorToast).toHaveBeenCalled());
  expect(successToast).not.toHaveBeenCalled();
  expect(errorToast.mock.calls[0]?.[1]?.description).toMatch(CORE_MEMBER_ID);
});

/** Rollback residuals remain an error and are named in the structured Pack feedback. */
it("reports Pack rollback residuals in the error toast", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithPack();
  state.installOutcome = {
    state: "pack_installed",
    members: [],
    skipped: [],
    failed: {
      pluginId: LINT_MEMBER_ID,
      errorCode: "plugin_download_failed",
      rollbackFailures: [
        { pluginId: CORE_MEMBER_ID, errorCode: "plugin_uninstall_failed" },
      ],
    },
  };
  const errorToast = vi.spyOn(toast, "error").mockImplementation(() => "toast");
  renderSettings(client);

  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));

  await waitFor(() => expect(errorToast).toHaveBeenCalled());
  expect(errorToast.mock.calls[0]?.[1]?.description).toMatch(CORE_MEMBER_ID);
});

/** Marketplace card and README installs consume the same Pack outcome interpreter. */
it("shows identical Pack failure feedback from card and README installs", async () => {
  const failure: InstallOutcome = {
    state: "pack_installed",
    members: [],
    skipped: [],
    failed: {
      pluginId: CORE_MEMBER_ID,
      errorCode: "plugin_download_failed",
      rollbackFailures: [],
    },
  };
  const errorToast = vi
    .spyOn(toast, "error")
    .mockClear()
    .mockImplementation(() => "toast");
  const cardFixture = clientWithPack();
  cardFixture.state.installOutcome = failure;
  const card = renderSettings(cardFixture.client);
  const cardUser = userEvent.setup();
  await cardUser.click(
    await screen.findByRole("button", { name: /安装|Install/ }),
  );
  await waitFor(() => expect(errorToast).toHaveBeenCalledOnce());
  const cardFeedback = errorToast.mock.calls[0];
  card.unmount();
  act(() => usePluginOperationStore.setState({ activities: {} }));
  errorToast.mockClear();

  const detailFixture = clientWithPack();
  detailFixture.state.installOutcome = failure;
  renderSettings(detailFixture.client);
  const detailUser = userEvent.setup();
  await detailUser.click(await screen.findByText("Python Extension Pack"));
  await detailUser.click(
    await screen.findByRole("button", { name: /安装|Install/ }),
  );

  await waitFor(() => expect(errorToast).toHaveBeenCalledOnce());
  expect(errorToast.mock.calls[0]).toEqual(cardFeedback);
});

/** The ownership plan is a hard gate while its backend query is unresolved. */
it("disables Pack uninstall confirmation while the plan is loading", async () => {
  const user = userEvent.setup();
  const { state, handlers, client } = clientWithPack();
  seedInstalledPack(state, [
    {
      memberId: CORE_MEMBER_ID,
      versionAtInstall: "1.0.0",
      ownership: "managed_by_pack",
      state: { state: "expected_and_present" },
    },
  ]);
  vi.spyOn(handlers, "packUninstallPlan").mockImplementation(
    () => new Promise<never>(() => undefined),
  );
  renderSettings(client);

  await user.click(
    await screen.findByRole("button", { name: /卸载集合包|Uninstall pack/ }),
  );

  const dialog = await screen.findByRole("alertdialog");
  expect(
    within(dialog).getByRole("button", { name: /确认卸载|Uninstall/ }),
  ).toBeDisabled();
  expect(
    within(dialog).getByText(/正在计算卸载范围|Calculating uninstall scope/),
  ).toBeInTheDocument();
});

/** A failed plan query stays visible and cannot fall through to destructive execution. */
it("shows a Pack uninstall plan error and keeps confirmation disabled", async () => {
  const user = userEvent.setup();
  const { state, handlers, client } = clientWithPack();
  seedInstalledPack(state, []);
  vi.spyOn(handlers, "packUninstallPlan").mockRejectedValue(
    new Error("plan unavailable"),
  );
  renderSettings(client);

  await user.click(
    await screen.findByRole("button", { name: /卸载集合包|Uninstall pack/ }),
  );

  const dialog = await screen.findByRole("alertdialog");
  expect(
    await within(dialog).findByText(
      /无法加载集合包卸载计划|Unable to load the pack uninstall plan/,
    ),
  ).toBeInTheDocument();
  expect(
    within(dialog).getByRole("button", { name: /确认卸载|Uninstall/ }),
  ).toBeDisabled();
});

/** Backend uninstall failure remains visible and leaves the Pack presentation intact. */
it("reports a Pack uninstall failure without dismissing the dialog", async () => {
  const user = userEvent.setup();
  const { state, handlers, client } = clientWithPack();
  seedInstalledPack(state, [
    {
      memberId: CORE_MEMBER_ID,
      versionAtInstall: "1.0.0",
      ownership: "managed_by_pack",
      state: { state: "expected_and_present" },
    },
  ]);
  vi.spyOn(handlers, "uninstallPlugin").mockRejectedValue(
    new Error("member is locked"),
  );
  const errorToast = vi.spyOn(toast, "error").mockImplementation(() => "toast");
  renderSettings(client);
  await user.click(
    await screen.findByRole("button", { name: /卸载集合包|Uninstall pack/ }),
  );
  const dialog = await screen.findByRole("alertdialog");
  await user.click(
    await within(dialog).findByRole("button", {
      name: /确认卸载|Uninstall/,
    }),
  );

  await waitFor(() => expect(errorToast).toHaveBeenCalled());
  expect(dialog).toBeInTheDocument();
  expect(state.packInstallations).toHaveLength(1);
});

/** Pack uninstall removes managed members but preserves packages that predate the relationship. */
it("preserves a pre-existing member when uninstalling a Pack", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithPack();
  state.installedPlugins = [
    packMember(CORE_MEMBER_ID),
    packMember(LINT_MEMBER_ID),
  ];
  seedInstalledPack(state, [
    {
      memberId: CORE_MEMBER_ID,
      versionAtInstall: "1.0.0",
      ownership: "pre_existing",
      state: { state: "expected_and_present" },
    },
    {
      memberId: LINT_MEMBER_ID,
      versionAtInstall: "1.0.0",
      ownership: "managed_by_pack",
      state: { state: "expected_and_present" },
    },
  ]);
  renderSettings(client);
  await user.click(
    await screen.findByRole("button", { name: /卸载集合包|Uninstall pack/ }),
  );
  const dialog = await screen.findByRole("alertdialog");
  await user.click(
    await within(dialog).findByRole("button", {
      name: /确认卸载|Uninstall/,
    }),
  );

  await waitFor(() => expect(state.packInstallations).toHaveLength(0));
  expect(state.installedPlugins.map((plugin) => plugin.id)).toEqual([
    CORE_MEMBER_ID,
  ]);
  await waitFor(() =>
    expect(
      screen.queryByRole("heading", { name: /已安装集合包|Installed packs/ }),
    ).not.toBeInTheDocument(),
  );
});

/** Marketplace cards expose native byte progress while a package download is pending. */
it("shows marketplace plugin download progress", async () => {
  const user = userEvent.setup();
  const { client, handlers } = clientWithWeather();
  vi.spyOn(handlers, "installPlugin").mockImplementation(
    () => new Promise<never>(() => undefined),
  );
  let reportProgress:
    | ((progress: {
        pluginId: string;
        downloaded: number;
        total: number | null;
      }) => void)
    | undefined;
  const platform: PlatformAdapter = {
    ...createStubPlatform(),
    pluginMarketplace: {
      onInstallProgress: async (listener) => {
        reportProgress = listener;
        return () => undefined;
      },
      onAutoSyncChanged: async () => () => undefined,
    },
  };
  renderSettings(client, platform);

  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));
  act(() => {
    reportProgress?.({
      pluginId: "official/weather",
      downloaded: 4,
      total: 10,
    });
  });

  expect(
    await screen.findByRole("progressbar", {
      name: /插件下载进度|Plugin download progress/,
    }),
  ).toHaveAttribute("aria-valuenow", "40");
  expect(screen.queryByText("40%")).not.toBeInTheDocument();
  expect(document.querySelector('[data-slot="progress"]')).toBeNull();
});

/** Marketplace updates reuse the durable byte-progress presentation used by installs. */
it("shows marketplace plugin update download progress", async () => {
  const user = userEvent.setup();
  const { state, client, handlers } = clientWithWeather();
  state.installedPlugins.push({
    ...weatherInstalled(),
    version: "1.1.0",
  });
  vi.spyOn(handlers, "updatePlugin").mockImplementation(
    () => new Promise<never>(() => undefined),
  );
  let reportProgress:
    | ((progress: {
        pluginId: string;
        downloaded: number;
        total: number | null;
      }) => void)
    | undefined;
  const platform: PlatformAdapter = {
    ...createStubPlatform(),
    pluginMarketplace: {
      onInstallProgress: async (listener) => {
        reportProgress = listener;
        return () => undefined;
      },
      onAutoSyncChanged: async () => () => undefined,
    },
  };
  renderSettings(client, platform);

  await user.click(await screen.findByRole("button", { name: /更新|Update/ }));
  act(() => {
    reportProgress?.({
      pluginId: "official/weather",
      downloaded: 3,
      total: 4,
    });
  });

  const progress = await screen.findByRole("progressbar", {
    name: /插件下载进度|Plugin download progress/,
  });
  expect(progress).toHaveAttribute("aria-valuenow", "75");
  const updateIcon = progress.querySelector(".tabler-icon-arrow-big-up-lines");
  expect(updateIcon).toBeInTheDocument();
  expect(updateIcon).not.toHaveClass("animate-bounce");
});

/** The installed-plugin update action stays visually light instead of drawing a box around it. */
it("renders the managed plugin update action without an outline", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  state.installedPlugins.push({
    ...weatherInstalled(),
    version: "1.1.0",
  });
  renderSettings(client);

  await openManagePlugins(user);

  expect(
    await screen.findByRole("button", { name: /更新|Update/ }),
  ).not.toHaveClass("border-border");
});

/** The managed update action names the active operation instead of retaining its idle label. */
it("labels a managed plugin update as updating while it downloads", async () => {
  const user = userEvent.setup();
  const { state, client, handlers } = clientWithWeather();
  state.installedPlugins.push({
    ...weatherInstalled(),
    version: "1.1.0",
  });
  vi.spyOn(handlers, "updatePlugin").mockImplementation(
    () => new Promise<never>(() => undefined),
  );
  renderSettings(client);
  await openManagePlugins(user);

  await user.click(await screen.findByRole("button", { name: /更新|Update/ }));

  expect(
    await screen.findByRole("button", { name: /更新中|Updating/ }),
  ).toBeDisabled();
});

/** A sync control pulls the marketplace source through the backend. */
it("syncs the marketplace through the backend", async () => {
  const user = userEvent.setup();
  const { client } = clientWithWeather();
  const syncSpy = vi.spyOn(client.plugin, "syncAvailable");
  renderSettings(client);

  await user.click(
    await screen.findByRole("button", {
      name: /同步插件市场|Sync marketplace/,
    }),
  );

  await waitFor(() => expect(syncSpy).toHaveBeenCalled());
});

/** A failed marketplace sync surfaces an error toast instead of failing silently. */
it("reports a failed marketplace sync", async () => {
  const user = userEvent.setup();
  const { client, handlers } = clientWithWeather();
  vi.spyOn(handlers, "syncAvailablePlugins").mockRejectedValue(
    new Error("marketplace unreachable"),
  );
  const errorToast = vi
    .spyOn(toast, "error")
    .mockClear()
    .mockImplementation(() => "toast");
  renderSettings(client);

  await user.click(
    await screen.findByRole("button", {
      name: /同步插件市场|Sync marketplace/,
    }),
  );

  await waitFor(() => expect(errorToast).toHaveBeenCalled());
  expect(errorToast.mock.calls[0]?.[0]).toEqual(
    expect.stringMatching(
      /同步插件市场失败|Failed to sync the plugin marketplace/,
    ),
  );
});

/** Importing a local archive goes through the backend and commits an enabled package. */
it("imports a local archive through the backend", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  state.importTarget = weatherInstalled();
  const platform = createStubPlatform();
  platform.selectPath = vi.fn().mockResolvedValue("C:/downloads/weather.orax");
  const importSpy = vi.spyOn(client.plugin, "import");
  const successToast = vi
    .spyOn(toast, "success")
    .mockClear()
    .mockImplementation(() => "toast");
  renderSettings(client, platform);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", { name: /导入插件|Import plugin/ }),
  );

  await waitFor(() =>
    expect(importSpy).toHaveBeenCalledWith({
      path: "C:/downloads/weather.orax",
      hookExecutionAcknowledged: false,
    }),
  );
  await waitFor(() => expect(state.installedPlugins).toHaveLength(1));
  expect(state.installedPlugins[0]).toMatchObject({
    id: "official/weather",
  });
  expect(successToast).toHaveBeenCalledWith(
    expect.stringMatching(/插件已导入|Plugin imported/),
  );
});

/** A path picker that rejects surfaces an error toast without touching the backend. */
it("reports a path-picker failure when importing", async () => {
  const user = userEvent.setup();
  const { client } = clientWithWeather();
  const platform = createStubPlatform();
  platform.selectPath = vi.fn().mockRejectedValue(new Error("picker closed"));
  const errorToast = vi
    .spyOn(toast, "error")
    .mockClear()
    .mockImplementation(() => "toast");
  renderSettings(client, platform);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", { name: /导入插件|Import plugin/ }),
  );

  await waitFor(() => expect(errorToast).toHaveBeenCalled());
  expect(errorToast.mock.calls[0]?.[0]).toEqual(
    expect.stringMatching(/无法选择插件文件|Unable to select a plugin file/),
  );
});

/** A registry entry's own brand mark is drawn as an inert image instead of the generic mark. */
it("renders the brand mark shipped with a marketplace plugin", async () => {
  const { client } = clientWithWeather(WEATHER_LOGO);
  const { container } = renderSettings(client);

  await screen.findByText("Weather");
  const logo = container.querySelector("img");
  expect(logo).toHaveAttribute("src", WEATHER_LOGO.url);
});

/** Plugins that ship no mark keep the row shape by falling back to the generic plug icon. */
it("falls back to the generic mark when a plugin ships no logo", async () => {
  const { client } = clientWithWeather();
  const { container } = renderSettings(client);

  await screen.findByText("Weather");
  expect(container.querySelector("img")).toBeNull();
});

/** The installed manager surfaces the logo carried by the installed package. */
it("renders the brand mark of an installed plugin in the manager", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather(WEATHER_LOGO);
  const { container } = renderSettings(client);

  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));
  await waitFor(() => expect(state.installedPlugins).toHaveLength(1));
  await openManagePlugins(user);

  await screen.findByText("official/weather");
  expect(container.querySelector("img")).toHaveAttribute(
    "src",
    WEATHER_LOGO.url,
  );
  expect(
    screen.queryByRole("button", { name: /启动|Start/ }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /停止|Stop/ }),
  ).not.toBeInTheDocument();
});

/** Installed plugins no longer expose start or stop, regardless of runtime. */
it("hides start and stop for stopped, starting, failed, and running plugins", async () => {
  const user = userEvent.setup();
  const state = createFixtureState();
  state.installedPlugins = [
    weatherInstalled(),
    { ...weatherInstalled(), id: "official/starting", runtime: "starting" },
    {
      ...weatherInstalled(),
      id: "official/failed",
      runtime: "failed",
      failureReason: "launch failed",
    },
    { ...weatherInstalled(), id: "official/running", runtime: "running" },
  ];
  renderSettings(createTestClient(createFixtureHandlers(state)));

  await openManagePlugins(user);
  await screen.findByText("official/weather");

  expect(
    screen.queryByRole("button", { name: /启动|Start/ }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /启动中|Starting/ }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /停止|Stop/ }),
  ).not.toBeInTheDocument();
});

/** Host-rendered fields preserve defaults and explicit boolean false through Save. */
it("configures declared plugin settings and keeps the editor open after save", async () => {
  const user = userEvent.setup();
  const state = createFixtureState();
  state.installedPlugins.push({
    ...weatherInstalled(),
    configuration: { state: "available", completeness: "incomplete" },
  });
  state.pluginConfigurations.set("official/weather", {
    pluginId: "official/weather",
    schemaVersion: 1,
    revision: 0n,
    declarationFingerprint: "declaration-1",
    settings: [
      {
        declaration: {
          id: "endpoint",
          title: "Endpoint",
          description: "Service URL",
          type: "string",
          required: true,
          order: 1n,
          default: null,
        },
        storedValue: null,
        effectiveValue: null,
        redacted: false,
        source: "absent",
        valueErrorCode: null,
      },
      {
        declaration: {
          id: "retries",
          title: "Retries",
          description: "Attempts",
          type: "number",
          required: false,
          order: null,
          default: 3,
        },
        storedValue: null,
        effectiveValue: 3,
        redacted: false,
        source: "default",
        valueErrorCode: null,
      },
      {
        declaration: {
          id: "enabled",
          title: "Enabled",
          description: "Use it",
          type: "boolean",
          required: false,
          order: null,
          default: null,
        },
        storedValue: null,
        effectiveValue: null,
        redacted: false,
        source: "absent",
        valueErrorCode: null,
      },
    ],
    summary: { state: "available", completeness: "incomplete" },
  });
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  const save = vi.spyOn(client.plugin, "saveConfiguration");
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", { name: /配置|Configure/ }),
  );
  await user.type(await screen.findByLabelText(/Endpoint/), "https://api.test");
  expect(screen.getByLabelText(/Retries/)).toHaveValue("3");
  await user.selectOptions(screen.getByLabelText(/Enabled/), "false");
  await user.click(screen.getByRole("button", { name: /保存|Save/ }));

  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0]?.[0].values).toEqual({
    endpoint: "https://api.test",
    enabled: false,
  });
  expect(await screen.findByText(/已保存|Saved/)).toBeInTheDocument();
  expect(screen.getByLabelText(/Endpoint/)).toHaveValue("https://api.test");
});

/** Resetting one persisted field removes its override instead of restoring the same stored draft. */
it("removes an existing stored override when one field is reset and saved", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithPluginConfiguration();
  const configuration = state.pluginConfigurations.get("official/weather");
  if (configuration === undefined)
    throw new Error("configuration fixture missing");
  configuration.settings[0] = {
    ...configuration.settings[0]!,
    storedValue: "https://old.test",
    effectiveValue: "https://old.test",
    source: "stored",
  };
  const save = vi.spyOn(client.plugin, "saveConfiguration");
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", { name: /配置|Configure/ }),
  );
  await user.click(
    await screen.findByRole("button", { name: /重置此项|Reset field/ }),
  );
  await user.click(screen.getByRole("button", { name: /保存|Save/ }));

  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0]?.[0].values).toEqual({});
});

/** Back navigation cannot silently discard a local configuration draft. */
it("requires an explicit decision before leaving a dirty configuration editor", async () => {
  const user = userEvent.setup();
  const { client } = clientWithPluginConfiguration();
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", { name: /配置|Configure/ }),
  );
  await user.type(await screen.findByLabelText(/Endpoint/), "draft");
  await user.click(
    screen.getByRole("button", { name: /管理插件|Manage plugins/ }),
  );

  const dialog = await screen.findByRole("alertdialog");
  expect(
    within(dialog).getByText(/保存配置更改|Save configuration changes/),
  ).toBeInTheDocument();
  await user.click(within(dialog).getByRole("button", { name: /取消|Cancel/ }));
  expect(screen.getByLabelText(/Endpoint/)).toHaveValue("draft");
});

/** A stale editor keeps its local input until the user reloads the latest save baseline. */
it("preserves a configuration draft when the declaration changes during save", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithPluginConfiguration();
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", { name: /配置|Configure/ }),
  );
  await user.type(await screen.findByLabelText(/Endpoint/), "draft");

  const latest = state.pluginConfigurations.get("official/weather");
  if (latest === undefined) throw new Error("configuration fixture missing");
  state.pluginConfigurations.set("official/weather", {
    ...latest,
    revision: 1n,
    declarationFingerprint: "declaration-2",
  });

  await user.click(screen.getByRole("button", { name: /保存|Save/ }));
  expect(
    await screen.findByText(
      /配置已在其他位置更新|Configuration changed elsewhere/,
    ),
  ).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: /重新加载|Reload/ }));
  await waitFor(() =>
    expect(screen.getByLabelText(/Endpoint/)).toHaveValue("draft"),
  );
});

/** Damaged storage needs a second confirmation before the recovery domain operation runs. */
it("confirms corrupt configuration recovery before replacing values", async () => {
  const user = userEvent.setup();
  const { client } = clientWithPluginConfiguration(true);
  const reset = vi.spyOn(client.plugin, "resetConfiguration");
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", { name: /配置|Configure/ }),
  );
  await user.click(
    await screen.findByRole("button", {
      name: /备份并恢复|Back up and recover/,
    }),
  );
  expect(reset).not.toHaveBeenCalled();

  const dialog = await screen.findByRole("alertdialog");
  await user.click(
    within(dialog).getByRole("button", {
      name: /备份并恢复|Back up and recover/,
    }),
  );

  await waitFor(() =>
    expect(reset).toHaveBeenCalledWith({
      pluginId: "official/weather",
      declarationFingerprint: "declaration-1",
      mode: "recover_corrupt",
    }),
  );
  expect(await screen.findByLabelText(/Endpoint/)).toBeInTheDocument();
});

/** Host-incompatible marketplace listings keep Install disabled and explain why. */
it("disables install for a host-incompatible marketplace plugin", async () => {
  const state = createFixtureState();
  state.availablePlugins.push({
    id: "official/rtk-ai.rtk",
    name: "rtk-ai.rtk",
    title: "RTK",
    kind: "hook",
    namespace: "official",
    sourceUrl: "https://github.com/ora-space/marketplace",
    version: "0.1.0",
    description: "RTK command rewrite hook",
    logo: null,
    compatibility: "incompatible",
    reason:
      "this release supports x86_64-pc-windows-msvc but your host is aarch64-apple-darwin",
  });
  renderSettings(createTestClient(createFixtureHandlers(state)));

  expect(await screen.findByText("RTK")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /安装|Install/ })).toBeDisabled();
  expect(
    screen.getByText(
      /this release supports x86_64-pc-windows-msvc but your host is aarch64-apple-darwin/,
    ),
  ).toBeInTheDocument();
});

/** A Hook package without Settings has no Configure action and surfaces its descriptor. */
it("shows hook descriptor fields and hides configure when settings are not declared", async () => {
  const user = userEvent.setup();
  const state = createFixtureState();
  state.installedPlugins.push(hookInstalled());
  renderSettings(createTestClient(createFixtureHandlers(state)));

  await openManagePlugins(user);
  expect(await screen.findByText("official/rtk-ai.rtk")).toBeInTheDocument();
  expect(
    screen.getByText(
      /0\.1\.0 · hook · stopped · assets\/rtk\.exe · claude-code, codex · x86_64-pc-windows-msvc/,
    ),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /配置|Configure/ }),
  ).not.toBeInTheDocument();
});

/** A Hook install discloses the program it will run before the request may carry the grant. */
it("discloses hook execution before installing a marketplace hook", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithHook();
  const installSpy = vi.spyOn(client.plugin, "install");
  renderSettings(client);

  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));

  const dialog = await screen.findByRole("alertdialog", {
    name: /执行.*包含的程序|Run the program bundled with/,
  });
  expect(dialog).toHaveTextContent(
    /此插件会执行包内程序，并可能读取或修改用户文件以及 Agent 配置文件|executes a program from its package/,
  );
  expect(installSpy).not.toHaveBeenCalled();

  await user.click(
    within(dialog).getByRole("button", { name: /安装并执行|Install and run/ }),
  );

  await waitFor(() => expect(installSpy).toHaveBeenCalledOnce());
  expect(installSpy.mock.calls[0]?.[0]).toEqual({
    pluginId: HOOK_ID,
    hookExecutionAcknowledged: true,
  });
  await waitFor(() => expect(state.hookLifecycleReports.size).toBe(1));
  // The confirmation is answered from the card, so it must not also open the detail page.
  expect(
    screen.getByRole("button", { name: /查看 RTK 的 README|View RTK README/ }),
  ).toBeInTheDocument();
});

/** An update re-runs `init`, so it asks for the grant again rather than reusing the install one. */
it("discloses hook execution again before updating an installed hook", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithHook("0.2.0");
  state.installedPlugins = [hookInstalled("0.1.0")];
  const updateSpy = vi.spyOn(client.plugin, "update");
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(await screen.findByRole("button", { name: /更新|Update/ }));

  const dialog = await screen.findByRole("alertdialog", {
    name: /执行.*包含的程序|Run the program bundled with/,
  });
  expect(updateSpy).not.toHaveBeenCalled();

  await user.click(
    within(dialog).getByRole("button", { name: /更新并执行|Update and run/ }),
  );

  await waitFor(() => expect(updateSpy).toHaveBeenCalledOnce());
  expect(updateSpy.mock.calls[0]?.[0]).toEqual({
    pluginId: HOOK_ID,
    hookExecutionAcknowledged: true,
  });
  expect(
    await screen.findByText(/本次会话已初始化|Initialized this session/),
  ).toBeInTheDocument();
});

/** With no result this session a Hook reads as unknown, and initializing it is an explicit act. */
it("shows an uninitialized hook and initializes it on request", async () => {
  const user = userEvent.setup();
  const { client } = clientWithInstalledHook();
  renderSettings(client);

  await openManagePlugins(user);
  expect(
    await screen.findByText(/本次会话未初始化|Not initialized this session/),
  ).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: /初始化|Initialize/ }));

  expect(
    await screen.findByText(/本次会话已初始化|Initialized this session/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/重启对应 Agent|restart the Agent/),
  ).toBeInTheDocument();
});

/** A failed initialization stays visible with its diagnostic and remains retryable by the user. */
it("keeps a failed hook initialization visible and retries it", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithInstalledHook();
  state.hookOutcome = {
    state: "failed",
    exitCode: 1,
    durationMs: 9,
    reason: "the command exited with code 1",
  };
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(screen.getByRole("button", { name: /初始化|Initialize/ }));

  expect(
    await screen.findByText(/初始化失败|Initialization failed/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/the command exited with code 1/),
  ).toBeInTheDocument();
  expect(state.installedPlugins).toHaveLength(1);

  state.hookOutcome = undefined;
  await user.click(screen.getByRole("button", { name: /初始化|Initialize/ }));

  expect(
    await screen.findByText(/本次会话已初始化|Initialized this session/),
  ).toBeInTheDocument();
});

/** Removing a Hook states what runs and what stays behind, and its confirmation is the grant. */
it("states the hook teardown before uninstalling and authorizes it once confirmed", async () => {
  const user = userEvent.setup();
  const { client } = clientWithInstalledHook();
  const uninstallSpy = vi.spyOn(client.plugin, "uninstall");
  renderSettings(client);

  await openManagePlugins(user);
  await user.click(
    await screen.findByRole("button", {
      name: /打开 rtk-ai\.rtk 的菜单|Open the rtk-ai\.rtk menu/,
    }),
  );
  await user.click(
    await screen.findByRole("menuitem", { name: /卸载|Uninstall/ }),
  );

  const dialog = await screen.findByRole("alertdialog", {
    name: /卸载.*rtk-ai\.rtk|Uninstall rtk-ai\.rtk/,
  });
  expect(dialog).toHaveTextContent(
    /卸载会先执行该工具声明的反初始化命令|Uninstalling runs the tool's declared teardown/,
  );

  await user.click(
    within(dialog).getByRole("button", { name: /^卸载$|^Uninstall$/ }),
  );

  await waitFor(() => expect(uninstallSpy).toHaveBeenCalledOnce());
  expect(uninstallSpy.mock.calls[0]?.[0]).toEqual({
    pluginId: HOOK_ID,
    dataDisposition: "delete",
    hookExecutionAcknowledged: true,
  });
});

/** The header gear offers the manage-plugin and manage-marketplace destinations. */
it("opens the management menu from the marketplace header", async () => {
  const user = userEvent.setup();
  const { client } = clientWithWeather();
  renderSettings(client);

  await user.click(
    await screen.findByRole("button", {
      name: /插件管理操作|Plugin management actions/,
    }),
  );

  expect(
    await screen.findByRole("menuitem", { name: /管理插件|Manage plugins/ }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("menuitem", { name: /管理市场|Manage marketplace/ }),
  ).toBeInTheDocument();
});

/** Clicking a marketplace card opens its README detail page rendered from the backend. */
it("opens the README page when a marketplace card is clicked", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  state.pluginReadmes.set(
    "official/weather",
    "# Weather\n\nLive forecasts every hour.",
  );
  renderSettings(client);

  await user.click(await screen.findByText("Weather"));

  expect(
    await screen.findByText("Live forecasts every hour."),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("heading", { level: 1, name: "Weather" }),
  ).toBeInTheDocument();
});

/** An uninstalled listing keeps the marketplace install command available on its detail page. */
it("installs an uninstalled plugin from its detail header", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  renderSettings(client);

  await user.click(await screen.findByText("Weather"));
  await user.click(await screen.findByRole("button", { name: /安装|Install/ }));

  await waitFor(() => expect(state.installedPlugins).toHaveLength(1));
  expect(
    await screen.findByRole("button", { name: /卸载|Uninstall/ }),
  ).toBeInTheDocument();
});

/** An older installed release exposes update, then changes to uninstall after refreshing. */
it("updates an outdated plugin from its detail header", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  state.installedPlugins.push({ ...weatherInstalled(), version: "1.1.0" });
  renderSettings(client);

  await user.click(await screen.findByText("Weather"));
  await user.click(await screen.findByRole("button", { name: /更新|Update/ }));

  await waitFor(() => expect(state.installedPlugins[0]?.version).toBe("1.2.0"));
  expect(
    await screen.findByRole("button", { name: /卸载|Uninstall/ }),
  ).toBeInTheDocument();
});

/** A current installed release uses the existing confirmation flow before uninstalling. */
it("uninstalls a current plugin from its detail header", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  state.installedPlugins.push(weatherInstalled());
  renderSettings(client);

  await user.click(await screen.findByText("Weather"));
  await user.click(
    await screen.findByRole("button", { name: /卸载|Uninstall/ }),
  );
  const dialog = await screen.findByRole("alertdialog");
  await user.click(
    within(dialog).getByRole("button", { name: /卸载|Uninstall/ }),
  );

  await waitFor(() => expect(state.installedPlugins).toHaveLength(0));
  expect(
    await screen.findByRole("button", { name: /安装|Install/ }),
  ).toBeInTheDocument();
});

/** The README page breadcrumb returns to the marketplace grid. */
it("returns from the README page to the marketplace grid", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  state.pluginReadmes.set("official/weather", "# Weather");
  renderSettings(client);

  await user.click(await screen.findByText("Weather"));
  await user.click(await screen.findByRole("button", { name: /插件|Plugins/ }));

  expect(await screen.findByText("Weather plugin")).toBeInTheDocument();
});

/** A listing with no README still opens its detail page and explains the absence. */
it("shows an empty state when a listing publishes no README", async () => {
  const user = userEvent.setup();
  const { state, client } = clientWithWeather();
  state.pluginReadmes.delete("official/weather");
  renderSettings(client);

  await user.click(await screen.findByText("Weather"));

  expect(
    await screen.findByText(/该插件没有提供 README|does not ship a README/),
  ).toBeInTheDocument();
});

/** Marketplace descriptions stay on one line and ellipsize instead of wrapping. */
it("keeps marketplace descriptions to a single truncated line", async () => {
  const { client } = clientWithWeather();
  renderSettings(client);

  const description = await screen.findByText("Weather plugin");
  expect(description).toHaveClass("truncate");
  expect(description).not.toHaveClass("line-clamp-2");
});

/**
 * The rebuild outlives this page, so leaving it mid-sync and coming back must not restore a
 * button that looks ready: pressing it again would start a second rebuild.
 */
it("keeps the sync action disabled across leaving and reopening the page", async () => {
  const user = userEvent.setup();
  const { client, handlers } = clientWithWeather();
  let settle: (() => void) | undefined;
  vi.spyOn(handlers, "syncAvailablePlugins").mockImplementation(
    () =>
      new Promise((resolve) => {
        settle = () => resolve({ updatedAt: 0n, plugins: [] });
      }),
  );
  const view = renderSettings(client);

  await user.click(
    await screen.findByRole("button", {
      name: /同步插件市场|Sync plugin marketplace/,
    }),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", {
        name: /同步插件市场|Sync plugin marketplace/,
      }),
    ).toBeDisabled(),
  );

  // Leaving the settings page tears the mutation down; the rebuild behind it keeps going.
  view.unmount();
  renderSettings(client);

  const reopened = await screen.findByRole("button", {
    name: /同步插件市场|Sync plugin marketplace/,
  });
  expect(reopened).toBeDisabled();

  await act(async () => {
    settle?.();
    await Promise.resolve();
  });

  await waitFor(() => expect(reopened).toBeEnabled());
});

/**
 * The host admits one marketplace rebuild at a time and discards the rest, so the Sync action
 * stands down while the host is refreshing on its own rather than letting a click be dropped.
 */
it("disables the sync action while the host is refreshing the marketplace", async () => {
  const { client } = clientWithWeather();
  renderSettings(client);

  const sync = await screen.findByRole("button", {
    name: /同步插件市场|Sync plugin marketplace/,
  });
  expect(sync).toBeEnabled();

  act(() => useMarketplaceSyncStore.getState().setHostRefreshing(true));

  await waitFor(() => expect(sync).toBeDisabled());
  expect(within(sync).getByText(/正在同步…|Syncing…/)).toBeInTheDocument();

  act(() => useMarketplaceSyncStore.getState().setHostRefreshing(false));

  await waitFor(() => expect(sync).toBeEnabled());
});
