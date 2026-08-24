import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import type { ContractsClient } from "@ora/contracts";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { PluginsSettings } from "./plugins-settings";

/** Renders plugin settings with isolated query and contracts-client state. */
function renderSettings(client: ContractsClient) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>
          <PluginsSettings />
        </AppI18nProvider>
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
    namespace: "official",
    version: "1.2.0",
    description: "Weather plugin",
    logo,
  });
  return { state, client: createMockClient(state) };
}

/** Seeds one installed plugin and its smallest editable declaration. */
function clientWithPluginConfiguration(unavailable = false) {
  const state = createMockClientState();
  state.installedPlugins.push({
    id: "official/weather",
    packageName: "weather",
    displayName: "weather",
    version: "1.2.0",
    kind: "agent",
    main: "main.js",
    agent: { displayName: "weather", contractVersion: 1 },
    enabled: false,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: unavailable
      ? { state: "unavailable", errorCode: "configuration_load_failed" }
      : { state: "available", completeness: "incomplete" },
    runtime: "stopped",
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
        source: "absent",
        valueErrorCode: null,
      },
    ],
    summary: unavailable
      ? { state: "unavailable", errorCode: "configuration_load_failed" }
      : { state: "available", completeness: "incomplete" },
  });
  return { state, client: createMockClient(state) };
}

/** The browse grid is driven entirely by the backend registry index. */
it("renders marketplace plugins from the registry index", async () => {
  const { client } = clientWithWeather();
  renderSettings(client);

  expect(await screen.findByText("weather")).toBeInTheDocument();
  expect(screen.getByText(/official · 1.2.0/)).toBeInTheDocument();
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
    packageName: "official/weather",
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

/** A registry entry's own brand mark is drawn as an inert image instead of the generic mark. */
it("renders the brand mark shipped with a marketplace plugin", async () => {
  const { client } = clientWithWeather(WEATHER_LOGO);
  const { container } = renderSettings(client);

  await screen.findByText("weather");
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

  await screen.findByText("weather");
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

/** Host-rendered fields preserve defaults and explicit boolean false through Save. */
it("configures declared plugin settings and keeps the editor open after save", async () => {
  const user = userEvent.setup();
  const state = createMockClientState();
  state.installedPlugins.push({
    id: "official/weather",
    packageName: "weather",
    displayName: "weather",
    version: "1.2.0",
    kind: "agent",
    main: "main.js",
    agent: { displayName: "weather", contractVersion: 1 },
    enabled: false,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "available", completeness: "incomplete" },
    runtime: "stopped",
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
        source: "absent",
        valueErrorCode: null,
      },
    ],
    summary: { state: "available", completeness: "incomplete" },
  });
  const client = createMockClient(state);
  const save = vi.spyOn(client.plugin, "saveConfiguration");
  renderSettings(client);

  await user.click(
    await screen.findByRole("button", { name: /管理插件|Manage plugins/ }),
  );
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

  await user.click(
    await screen.findByRole("button", { name: /管理插件|Manage plugins/ }),
  );
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

  await user.click(
    await screen.findByRole("button", { name: /管理插件|Manage plugins/ }),
  );
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

/** Damaged storage needs a second confirmation before the recovery domain operation runs. */
it("confirms corrupt configuration recovery before replacing values", async () => {
  const user = userEvent.setup();
  const { client } = clientWithPluginConfiguration(true);
  const reset = vi.spyOn(client.plugin, "resetConfiguration");
  renderSettings(client);

  await user.click(
    await screen.findByRole("button", { name: /管理插件|Manage plugins/ }),
  );
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
