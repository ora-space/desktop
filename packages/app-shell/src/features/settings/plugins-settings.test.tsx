import { render, screen, waitFor } from "@testing-library/react";
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
