import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  onlineManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { createAppQueryClient } from "../../state/query-client";
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
function renderProxy(
  client: ContractsClient,
  queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  }),
) {
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

it("restores every saved proxy field with a fresh query cache after remount", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "proxy.example",
    port: 3128,
    username: "alice",
    password: "saved-secret",
  };
  const client = createTestClient(createFixtureHandlers(state));
  const first = renderProxy(client);
  expect(await screen.findByLabelText(/主机|Host/)).toHaveValue(
    "proxy.example",
  );
  first.unmount();
  renderProxy(client);
  expect(await screen.findByLabelText(/主机|Host/)).toHaveValue(
    "proxy.example",
  );
  expect(screen.getByLabelText(/端口|Port/)).toHaveValue("3128");
  expect(screen.getByLabelText(/用户名|Username/)).toHaveValue("alice");
  expect(screen.getByLabelText(/密码|Password/)).toHaveValue("saved-secret");
});

it("blocks editing on read failure and restores saved values after retry", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "saved.example",
    port: 3128,
    username: "alice",
    password: "secret",
  };
  let failing = true;
  const handlers = createFixtureHandlers(state);
  handlers.getProxySettings = async () => {
    if (failing) throw new Error("simulated storage read failure");
    return { settings: state.proxySettings };
  };
  const user = userEvent.setup();
  renderProxy(createTestClient(handlers));
  await screen.findByRole("alert");
  expect(screen.queryByLabelText(/主机|Host/)).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /保存|Save/ }),
  ).not.toBeInTheDocument();
  expect(state.proxySettings?.host).toBe("saved.example");
  failing = false;
  await user.click(screen.getByRole("button", { name: /重试|Retry/ }));
  expect(await screen.findByLabelText(/主机|Host/)).toHaveValue(
    "saved.example",
  );
  expect(screen.getByLabelText(/密码|Password/)).toHaveValue("secret");
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
    "https://google.com",
  );
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );

  await waitFor(() =>
    expect(check).toHaveBeenCalledWith({
      url: "https://google.com",
      settings: {
        host: "127.0.0.1",
        port: 7890,
        username: null,
        password: null,
      },
    }),
  );
});
it("retains the draft and blocks actions during a production reconnect read failure", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "original.example",
    port: 3128,
    username: "alice",
    password: "secret",
  };
  let failing = false;
  const handlers = createFixtureHandlers(state);
  handlers.getProxySettings = async () => {
    if (failing) throw new Error("read failed");
    return { settings: state.proxySettings };
  };
  const client = createTestClient(handlers);
  const save = vi.spyOn(client.proxy, "set");
  const clear = vi.spyOn(client.proxy, "clear");
  const check = vi.spyOn(client.proxy, "check");
  renderProxy(client, createAppQueryClient());
  const user = userEvent.setup();
  const host = await screen.findByLabelText(/主机|Host/);
  await user.clear(host);
  await user.type(host, "draft.example");
  failing = true;
  await act(async () => {
    const clock = vi.spyOn(Date, "now").mockReturnValue(Date.now() + 31_000);
    try {
      onlineManager.setOnline(false);
      onlineManager.setOnline(true);
      await new Promise((resolve) => setTimeout(resolve, 0));
    } finally {
      clock.mockRestore();
    }
  });
  await screen.findByRole("alert", {}, { timeout: 5000 });
  expect(screen.getByLabelText(/主机|Host/)).toHaveValue("draft.example");
  for (const name of [/保存|Save/, /清空|Clear/, /验证代理|Check connection/]) {
    const button = screen.getByRole("button", { name });
    expect(button).toBeDisabled();
    await user.click(button);
  }
  expect(save).not.toHaveBeenCalled();
  expect(clear).not.toHaveBeenCalled();
  expect(check).not.toHaveBeenCalled();
  failing = false;
  await user.click(screen.getByRole("button", { name: /重试|Retry/ }));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  expect(screen.getByLabelText(/主机|Host/)).toHaveValue("draft.example");
  expect(screen.getByLabelText(/密码|Password/)).toHaveValue("secret");
  await user.click(screen.getByRole("button", { name: /保存|Save/ }));
  await waitFor(() => expect(state.proxySettings?.host).toBe("draft.example"));
});
