import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import type { ContractsClient } from "@ora/contracts";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createSettingsMemory,
  settingsHandlers,
} from "../../test/memory/settings";
import { ProxySettings } from "./proxy-settings";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return { ...createSettingsMemory() };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...settingsHandlers(state),
  };
}

void appI18n;

/** Renders the proxy editor with an isolated query client and contracts client. */
function renderProxy(client: ContractsClient) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>
          <ProxySettings />
        </AppI18nProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
}

it("saves proxy settings through the backend", async () => {
  const state = createFixtureState();
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  const user = userEvent.setup();

  renderProxy(client);

  await user.type(await screen.findByLabelText(/主机|Host/), "127.0.0.1");
  await user.type(screen.getByLabelText(/端口|Port/), "7890");
  await user.click(screen.getByRole("button", { name: /保存|Save/ }));

  await waitFor(() =>
    expect(state.proxySettings).toEqual({
      host: "127.0.0.1",
      port: 7890,
      username: null,
      password: null,
    }),
  );
});

it("clears saved proxy settings through the backend", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "127.0.0.1",
    port: 7890,
    username: null,
    password: null,
  };
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  const user = userEvent.setup();

  renderProxy(client);

  await user.click(await screen.findByRole("button", { name: /清空|Clear/ }));

  await waitFor(() => expect(state.proxySettings).toBeNull());
});

it("checks the current form proxy against a URL", async () => {
  const state = createFixtureState();
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  const check = vi.spyOn(client.proxy, "check");
  const user = userEvent.setup();

  renderProxy(client);

  await user.type(await screen.findByLabelText(/主机|Host/), "127.0.0.1");
  await user.type(screen.getByLabelText(/端口|Port/), "7890");
  await user.click(
    screen.getByRole("button", { name: /验证代理|Check connection/ }),
  );

  const dialog = await screen.findByRole("dialog");
  expect(within(dialog).getByLabelText(/探测网址|URL to check/)).toHaveValue(
    "https://example.com/",
  );
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );

  await waitFor(() =>
    expect(check).toHaveBeenCalledWith({
      url: "https://example.com/",
      settings: {
        host: "127.0.0.1",
        port: 7890,
        username: null,
        password: null,
      },
    }),
  );
});
