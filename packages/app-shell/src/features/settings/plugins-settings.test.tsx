import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import type { ContractsClient, InstalledPlugin } from "@ora/contracts";
import { toast } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { PluginsSettings } from "./plugins-settings";

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
            <PluginsSettings />
          </AppI18nProvider>
        </PlatformProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
}

/** The registry-supplied brand mark, already security-validated by the backend. */
const WEATHER_LOGO =
  '<svg xmlns="http://www.w3.org/2000/svg"><rect width="8"/></svg>';

function clientWithWeather(logo: string | null = null) {
  const state = createMockClientState();
  state.availablePlugins.push({
    id: "official/weather",
    name: "weather",
    title: "Weather",
    kind: "agent",
    namespace: "official",
    version: "1.2.0",
    description: "Weather plugin",
    logo,
  });
  return { state, client: createMockClient(state) };
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
    enabled: false,
    logo: null,
    runtime: "stopped",
  };
}

/** The browse grid is driven entirely by the backend registry index. */
it("renders marketplace plugins from the registry index", async () => {
  const { client } = clientWithWeather();
  renderSettings(client);

  expect(await screen.findByText("Weather")).toBeInTheDocument();
  expect(
    screen.getByText(/weather · official · agent · 1.2.0/),
  ).toBeInTheDocument();
  expect(screen.getByText("Weather plugin")).toBeInTheDocument();
});

/** Installing goes through the backend and refreshes the installed surface. */
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
  expect(
    await screen.findByRole("button", { name: /卸载|Uninstall/ }),
  ).toBeInTheDocument();
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
  const { client } = clientWithWeather();
  vi.spyOn(client.plugin, "syncAvailable").mockRejectedValue(
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

  await user.click(
    await screen.findByRole("button", { name: /导入插件|Import plugin/ }),
  );

  await waitFor(() =>
    expect(importSpy).toHaveBeenCalledWith({
      path: "C:/downloads/weather.orax",
    }),
  );
  await waitFor(() => expect(state.installedPlugins).toHaveLength(1));
  expect(state.installedPlugins[0]).toMatchObject({
    id: "official/weather",
    enabled: true,
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
  expect(logo).toHaveAttribute(
    "src",
    `data:image/svg+xml;charset=utf-8,${encodeURIComponent(WEATHER_LOGO)}`,
  );
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
  await user.click(
    screen.getByRole("button", { name: /管理插件|Manage plugins/ }),
  );

  await screen.findByText("official/weather");
  expect(container.querySelector("img")).toHaveAttribute(
    "src",
    `data:image/svg+xml;charset=utf-8,${encodeURIComponent(WEATHER_LOGO)}`,
  );
});
