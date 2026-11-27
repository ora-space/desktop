import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  onlineManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { createAppQueryClient } from "../../state/query-client";
import { settingsKeys } from "../../state/data/settings";
import { expect, it, vi } from "vitest";
import type { ContractsClient } from "@ora/contracts";
import { toast } from "@ora/ui";
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

it("shows the Windows WPAD limitation only while System is selected", async () => {
  const state = createFixtureState();
  renderProxy(createTestClient(createFixtureHandlers(state)));
  const user = userEvent.setup();
  const mode = await screen.findByLabelText(/代理模式|Proxy mode/);
  expect(
    screen.getByText(
      /自动检测时将直连.*WPAD|automatic detection enabled.*directly.*WPAD/,
    ),
  ).toBeVisible();
  await user.selectOptions(mode, "direct");
  expect(screen.queryByText(/WPAD/)).not.toBeInTheDocument();
});

it("loads a retained manual endpoint only when selecting Manual and can replace a corrupt one", async () => {
  const state = createFixtureState();
  state.proxyMode = "system";
  const handlers = createFixtureHandlers(state);
  const get = vi.fn<NonNullable<typeof handlers.getProxySettings>>(
    async ({ loadManual }) => {
      if (loadManual) throw new Error("invalid stored endpoint");
      return { mode: "system", settings: null };
    },
  );
  renderProxy(createTestClient({ ...handlers, getProxySettings: get }));
  const user = userEvent.setup();
  const mode = await screen.findByLabelText(/代理模式|Proxy mode/);
  expect(get.mock.calls.map(([request]) => request)).toEqual([{}]);
  await user.selectOptions(mode, "manual");
  expect(await screen.findByRole("alert")).toHaveTextContent(
    /无法读取保留的手动代理配置|Could not read the saved manual proxy/,
  );
  expect(get.mock.calls.at(-1)?.[0]).toEqual({ loadManual: true });
  await user.type(screen.getByLabelText(/主机|Host/), "proxy.example");
  await user.type(screen.getByLabelText(/端口|Port/), "7890");
  await user.click(screen.getByRole("button", { name: /^保存$|^Save$/ }));
  await waitFor(() => expect(state.proxySettings?.host).toBe("proxy.example"));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("defaults to system and keeps the saved endpoint when switching modes", async () => {
  const state = createFixtureState();
  const client = createTestClient(createFixtureHandlers(state));
  const user = userEvent.setup();
  const first = renderProxy(client);
  const modeLabel = /代理模式|Proxy mode/;
  expect(await screen.findByLabelText(modeLabel)).toHaveValue("system");
  await user.selectOptions(screen.getByLabelText(modeLabel), "manual");
  await user.type(screen.getByLabelText(/主机|Host/), "proxy.example");
  await user.type(screen.getByLabelText(/端口|Port/), "7890");
  await user.click(screen.getByRole("button", { name: /保存|Save/ }));
  await waitFor(() => expect(state.proxySettings?.host).toBe("proxy.example"));

  await user.selectOptions(screen.getByLabelText(modeLabel), "direct");
  await user.click(screen.getByRole("button", { name: /保存|Save/ }));
  await waitFor(() => expect(state.proxyMode).toBe("direct"));
  first.unmount();

  renderProxy(client);
  expect(await screen.findByLabelText(modeLabel)).toHaveValue("direct");
  await user.selectOptions(screen.getByLabelText(modeLabel), "manual");
  expect(screen.getByLabelText(/主机|Host/)).toHaveValue("proxy.example");
});

it("saves proxy settings through the backend", async () => {
  const state = createFixtureState();
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  const user = userEvent.setup();

  renderProxy(client);

  await user.selectOptions(
    await screen.findByLabelText(/代理模式|Proxy mode/),
    "manual",
  );
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
    screen.queryByRole("button", { name: /^保存$|^Save$/ }),
  ).not.toBeInTheDocument();
  expect(state.proxySettings?.host).toBe("saved.example");
  failing = false;
  await user.click(screen.getByRole("button", { name: /重试|Retry/ }));
  expect(await screen.findByLabelText(/主机|Host/)).toHaveValue(
    "saved.example",
  );
  expect(screen.getByLabelText(/密码|Password/)).toHaveValue("secret");
});

it.each(["save", "clear"])(
  "can %s after explicitly replacing an unreadable preference",
  async (action) => {
    const state = createFixtureState();
    const handlers = createFixtureHandlers(state);
    handlers.getProxySettings = () =>
      Promise.reject(new Error("unknown stored mode"));
    const client = createTestClient(handlers);
    const user = userEvent.setup();
    renderProxy(client);
    await screen.findByRole("alert");
    const write = vi.spyOn(client.proxy, action === "save" ? "set" : "clear");
    await user.click(
      screen.getByRole("button", {
        name: /替换已保存的代理配置|Replace saved proxy settings/,
      }),
    );
    expect(write).not.toHaveBeenCalled();
    const mode = screen.getByLabelText(/代理模式|Proxy mode/);
    if (action === "save") await user.selectOptions(mode, "direct");
    await user.click(
      screen.getByRole("button", {
        name: action === "save" ? /^保存$|^Save$/ : /^清空$|^Clear$/,
      }),
    );
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(state.proxyMode).toBe(action === "save" ? "direct" : "system");
    expect(write).toHaveBeenCalledTimes(1);
  },
);

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

it("resets persisted direct mode even without a saved manual endpoint", async () => {
  const state = createFixtureState();
  state.proxyMode = "direct";
  const client = createTestClient(createFixtureHandlers(state));
  const user = userEvent.setup();
  const first = renderProxy(client);

  expect(await screen.findByLabelText(/代理模式|Proxy mode/)).toHaveValue(
    "direct",
  );
  await user.click(screen.getByRole("button", { name: /清空|Clear/ }));
  await waitFor(() =>
    expect(screen.getByLabelText(/代理模式|Proxy mode/)).toHaveValue("system"),
  );
  first.unmount();
  renderProxy(client);
  expect(await screen.findByLabelText(/代理模式|Proxy mode/)).toHaveValue(
    "system",
  );
});

it("discards a manual draft when clearing an already-default configuration", async () => {
  const state = createFixtureState();
  const user = userEvent.setup();
  renderProxy(createTestClient(createFixtureHandlers(state)));

  const modeLabel = /代理模式|Proxy mode/;
  await user.selectOptions(await screen.findByLabelText(modeLabel), "manual");
  await user.type(screen.getByLabelText(/主机|Host/), "draft.example");
  await user.type(screen.getByLabelText(/端口|Port/), "7890");
  await user.click(screen.getByRole("button", { name: /清空|Clear/ }));

  await waitFor(() =>
    expect(screen.getByLabelText(modeLabel)).toHaveValue("system"),
  );
  await user.selectOptions(screen.getByLabelText(modeLabel), "manual");
  expect(screen.getByLabelText(/主机|Host/)).toHaveValue("");
  expect(screen.getByLabelText(/端口|Port/)).toHaveValue("");
  expect(state.proxySettings).toBeNull();
});

it("keeps the saved mode when an older background read completes", async () => {
  const state = createFixtureState();
  const handlers = createFixtureHandlers(state);
  const getSettings = handlers.getProxySettings!;
  let resolveRead: (() => void) | undefined;
  let delayRead = false;
  handlers.getProxySettings = async (request) => {
    const result = await getSettings(request);
    if (delayRead) {
      await new Promise<void>((resolve) => {
        resolveRead = resolve;
      });
    }
    return result;
  };
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const user = userEvent.setup();
  renderProxy(createTestClient(handlers), queryClient);
  const modeLabel = /代理模式|Proxy mode/;
  await user.selectOptions(await screen.findByLabelText(modeLabel), "direct");
  delayRead = true;
  act(() => {
    void queryClient.refetchQueries({ queryKey: settingsKeys.proxySettings });
  });
  await waitFor(() => expect(resolveRead).toBeDefined());
  await user.click(screen.getByRole("button", { name: /保存|Save/ }));
  await waitFor(() =>
    expect(queryClient.getQueryData(settingsKeys.proxySettings)).toMatchObject({
      mode: "direct",
    }),
  );
  await act(async () => {
    resolveRead!();
  });
  expect(queryClient.getQueryData(settingsKeys.proxySettings)).toMatchObject({
    mode: "direct",
  });
  expect(screen.getByLabelText(modeLabel)).toHaveValue("direct");
});

it("checks the current form proxy against a URL", async () => {
  const state = createFixtureState();
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  const check = vi.spyOn(client.proxy, "check");
  const user = userEvent.setup();

  renderProxy(client);

  await user.selectOptions(
    await screen.findByLabelText(/代理模式|Proxy mode/),
    "manual",
  );
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
  for (const name of [
    /^保存$|^Save$/,
    /清空|Clear/,
    /验证代理|Check connection/,
  ]) {
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

it.each(["proxy", "url"])(
  "keeps an open probe URL draft while a background %s read recovers",
  async (failedTarget) => {
    const state = createFixtureState();
    state.proxySettings = {
      host: "original.example",
      port: 3128,
      username: null,
      password: null,
    };
    let failing = false;
    const handlers = createFixtureHandlers(state);
    handlers.getProxySettings = async () => {
      if (failing && failedTarget === "proxy") throw new Error("read failed");
      return { settings: state.proxySettings };
    };
    handlers.getProxyCheckUrl = async () => {
      if (failing && failedTarget === "url") throw new Error("URL read failed");
      return { url: state.proxyCheckUrl };
    };
    const client = createTestClient(handlers);
    const check = vi.spyOn(client.proxy, "check");
    renderProxy(client, createAppQueryClient());
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: /验证代理|Check connection/ }),
    );
    const dialog = await screen.findByRole("dialog");
    const url = within(dialog).getByLabelText(/探测网址|URL to check/);
    await user.clear(url);
    await user.type(url, "https://custom.example/probe");
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
    await within(dialog).findByRole("alert", {}, { timeout: 5000 });
    expect(screen.getByRole("dialog")).toBe(dialog);
    expect(url).toHaveValue("https://custom.example/probe");
    const probe = within(dialog).getByRole("button", {
      name: /^验证$|^Check$/,
    });
    expect(probe).toBeDisabled();
    await user.click(probe);
    await user.click(url);
    await user.keyboard("{Enter}");
    expect(check).not.toHaveBeenCalled();
    failing = false;
    await user.click(
      within(dialog).getByRole("button", { name: /重试|Retry/ }),
    );
    await waitFor(() => expect(within(dialog).queryByRole("alert")).toBeNull());
    expect(url).toHaveValue("https://custom.example/probe");
    await user.click(probe);
    await waitFor(() =>
      expect(check).toHaveBeenCalledWith({
        url: "https://custom.example/probe",
        settings: state.proxySettings,
      }),
    );
  },
);

it("reports an unreachable probe as connectivity without changing saved settings", async () => {
  const state = createFixtureState();
  const saved = {
    host: "proxy.example",
    port: 3128,
    username: null,
    password: null,
  };
  state.proxySettings = saved;
  const handlers = createFixtureHandlers(state);
  handlers.checkProxySettings = async () => ({
    outcome: "unreachable",
    message: "Target blocked by network policy",
  });
  const client = createTestClient(handlers);
  const set = vi.spyOn(client.proxy, "set");
  const clear = vi.spyOn(client.proxy, "clear");
  const report = vi.spyOn(toast, "error");
  renderProxy(client);
  const user = userEvent.setup();
  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  const dialog = await screen.findByRole("dialog");
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );
  await waitFor(() =>
    expect(report).toHaveBeenCalledWith(
      expect.stringMatching(/不代表代理配置有误|settings may still be valid/),
      { description: "Target blocked by network policy" },
    ),
  );
  expect(state.proxySettings).toEqual(saved);
  expect(set).not.toHaveBeenCalled();
  expect(clear).not.toHaveBeenCalled();
  expect(dialog).toBeInTheDocument();
});

it("tests an edited URL using an unsaved proxy draft", async () => {
  const testUrl = "https://example.com/probe";
  const state = createFixtureState();
  const handlers = createFixtureHandlers(state);
  handlers.checkProxySettings = () =>
    Promise.resolve({ outcome: "reachable", status: 200, route: "manual" });
  const client = createTestClient(handlers);
  const check = vi.spyOn(client.proxy, "check");
  const save = vi.spyOn(client.proxy, "set");
  const user = userEvent.setup();
  renderProxy(client);
  await user.selectOptions(
    await screen.findByLabelText(/代理模式|Proxy mode/),
    "manual",
  );
  await user.type(screen.getByLabelText(/主机|Host/), "https://proxy.example");
  await user.type(screen.getByLabelText(/端口|Port/), "7890");
  await user.click(
    screen.getByRole("button", { name: /验证代理|Check connection/ }),
  );
  const dialog = await screen.findByRole("dialog");
  expect(
    within(dialog).getByText(/未保存的设置|uses unsaved settings/),
  ).toBeInTheDocument();
  const urlInput = within(dialog).getByLabelText(/探测网址|URL to check/);
  await user.clear(urlInput);
  await user.type(urlInput, testUrl);
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );
  await waitFor(() => expect(dialog).not.toBeInTheDocument());
  expect(check).toHaveBeenCalledWith({
    url: testUrl,
    settings: {
      host: "https://proxy.example",
      port: 7890,
      username: null,
      password: null,
    },
  });
  expect(save).not.toHaveBeenCalled();
});

it("warns when an HTTP success bypasses the manual proxy", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "proxy.example",
    port: 3128,
    username: null,
    password: null,
  };
  const handlers = createFixtureHandlers(state);
  handlers.checkProxySettings = () =>
    Promise.resolve({ outcome: "reachable", status: 200, route: "direct" });
  const warning = vi.spyOn(toast, "warning");
  const success = vi.spyOn(toast, "success");
  const user = userEvent.setup();
  renderProxy(createTestClient(handlers));
  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  const dialog = await screen.findByRole("dialog");
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );
  await waitFor(() =>
    expect(warning).toHaveBeenCalledWith(
      expect.stringMatching(/最终请求走了直连|connected directly/),
      { description: expect.stringMatching(/HTTP.*200/) },
    ),
  );
  expect(success).not.toHaveBeenCalled();
  expect(dialog).toBeInTheDocument();
});

it("shows the destination HTTP status without reporting a successful check", async () => {
  const state = createFixtureState();
  const handlers = createFixtureHandlers(state);
  handlers.checkProxySettings = () =>
    Promise.resolve({ outcome: "http_error", status: 404 });
  const report = vi.spyOn(toast, "error");
  const success = vi.spyOn(toast, "success");
  const user = userEvent.setup();
  renderProxy(createTestClient(handlers));

  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  const dialog = await screen.findByRole("dialog");
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );
  await waitFor(() =>
    expect(report).toHaveBeenCalledWith(
      expect.stringMatching(
        /未返回成功状态|did not return a successful status/,
      ),
      { description: expect.stringMatching(/HTTP.*404/) },
    ),
  );
  expect(success).not.toHaveBeenCalled();
  expect(dialog).toBeInTheDocument();
});

it("resets a cancelled empty probe URL before opening a new check", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "proxy.example",
    port: 3128,
    username: null,
    password: null,
  };
  renderProxy(createTestClient(createFixtureHandlers(state)));
  const user = userEvent.setup();
  const open = await screen.findByRole("button", {
    name: /验证代理|Check connection/,
  });
  await user.click(open);
  const dialog = await screen.findByRole("dialog");
  await user.clear(within(dialog).getByLabelText(/探测网址|URL to check/));
  expect(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  ).toBeDisabled();
  await user.click(within(dialog).getByRole("button", { name: /取消|Cancel/ }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await user.click(open);
  const reopened = await screen.findByRole("dialog");
  expect(within(reopened).getByLabelText(/探测网址|URL to check/)).toHaveValue(
    "https://google.com",
  );
  expect(
    within(reopened).getByRole("button", { name: /^验证$|^Check$/ }),
  ).toBeEnabled();
});

it("saves a confirmed target despite an unreachable result and restores it after remount", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "saved.example",
    port: 3128,
    username: "alice",
    password: "secret",
  };
  const handlers = createFixtureHandlers(state);
  handlers.checkProxySettings = async () => ({
    outcome: "unreachable",
    message: "offline",
  });
  const client = createTestClient(handlers);
  const user = userEvent.setup();
  const view = renderProxy(client);
  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  const dialog = await screen.findByRole("dialog");
  const input = within(dialog).getByLabelText(/探测网址|URL to check/);
  await user.clear(input);
  await user.type(input, "https://custom.example/check");
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );
  await waitFor(() =>
    expect(state.proxyCheckUrl).toBe("https://custom.example/check"),
  );
  await waitFor(() =>
    expect(
      within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
    ).toBeEnabled(),
  );
  await user.clear(input);
  await user.type(input, "https://cancelled.example/");
  await user.click(within(dialog).getByRole("button", { name: /取消|Cancel/ }));
  view.unmount();
  renderProxy(client);
  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  expect(
    within(await screen.findByRole("dialog")).getByLabelText(
      /探测网址|URL to check/,
    ),
  ).toHaveValue("https://custom.example/check");
  expect(state.proxySettings).toEqual({
    host: "saved.example",
    port: 3128,
    username: "alice",
    password: "secret",
  });
});

it("preserves the URL draft and saved target on write failure, then retries before checking", async () => {
  const state = createFixtureState();
  state.proxyCheckUrl = "https://saved.example";
  state.proxySettings = {
    host: "proxy.example",
    port: 3128,
    username: null,
    password: null,
  };
  const handlers = createFixtureHandlers(state);
  const save = handlers.setProxyCheckUrl!;
  let failed = true;
  handlers.setProxyCheckUrl = async (request) => {
    if (failed) throw new Error("disk unavailable");
    return save(request);
  };
  const client = createTestClient(handlers);
  const check = vi.spyOn(client.proxy, "check");
  const error = vi.spyOn(toast, "error");
  const user = userEvent.setup();
  renderProxy(client);
  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  const dialog = await screen.findByRole("dialog");
  const input = within(dialog).getByLabelText(/探测网址|URL to check/);
  await user.clear(input);
  await user.type(input, "https://retry.example");
  const confirm = within(dialog).getByRole("button", {
    name: /^验证$|^Check$/,
  });
  await user.click(confirm);
  await waitFor(() =>
    expect(error).toHaveBeenCalledWith(
      expect.stringMatching(/网址未保存|URL was not saved/),
      expect.anything(),
    ),
  );
  expect(input).toHaveValue("https://retry.example");
  expect(state.proxyCheckUrl).toBe("https://saved.example");
  expect(check).not.toHaveBeenCalled();
  failed = false;
  await waitFor(() => expect(confirm).toBeEnabled());
  await user.click(confirm);
  await waitFor(() =>
    expect(check).toHaveBeenCalledWith({
      url: "https://retry.example",
      settings: state.proxySettings,
    }),
  );
});

it("does not replace an unreadable saved URL with the default and recovers on retry", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "proxy.example",
    port: 3128,
    username: null,
    password: null,
  };
  state.proxyCheckUrl = "https://saved.example/probe";
  const handlers = createFixtureHandlers(state);
  let failed = true;
  handlers.getProxyCheckUrl = async () => {
    if (failed) throw new Error("URL storage unavailable");
    return { url: state.proxyCheckUrl };
  };
  const client = createTestClient(handlers);
  const save = vi.spyOn(client.proxy, "setCheckUrl");
  renderProxy(client);
  await screen.findByRole("alert");
  expect(
    screen.getByRole("button", { name: /验证代理|Check connection/ }),
  ).toBeDisabled();
  expect(screen.getByRole("button", { name: /保存|Save/ })).toBeEnabled();
  expect(screen.getByLabelText(/主机|Host/)).toHaveValue("proxy.example");
  expect(save).not.toHaveBeenCalled();
  failed = false;
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: /重试|Retry/ }));
  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  expect(
    within(await screen.findByRole("dialog")).getByLabelText(
      /探测网址|URL to check/,
    ),
  ).toHaveValue("https://saved.example/probe");
});

it("does not let an older background response replace a newly confirmed target", async () => {
  const state = createFixtureState();
  state.proxySettings = {
    host: "proxy.example",
    port: 3128,
    username: null,
    password: null,
  };
  const handlers = createFixtureHandlers(state);
  let pending = false;
  let complete: ((value: { url: string }) => void) | undefined;
  handlers.getProxyCheckUrl = () =>
    pending
      ? new Promise<{ url: string }>((resolve) => {
          complete = resolve;
        })
      : Promise.resolve({ url: state.proxyCheckUrl });
  const client = createTestClient(handlers);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const user = userEvent.setup();
  renderProxy(client, queryClient);
  await user.click(
    await screen.findByRole("button", { name: /验证代理|Check connection/ }),
  );
  const dialog = await screen.findByRole("dialog");
  const input = within(dialog).getByLabelText(/探测网址|URL to check/);
  await user.clear(input);
  await user.type(input, "https://new.example/check");
  pending = true;
  act(() => {
    void queryClient.refetchQueries({ queryKey: settingsKeys.proxyCheckUrl });
  });
  await waitFor(() => expect(complete).toBeDefined());
  await user.click(
    within(dialog).getByRole("button", { name: /^验证$|^Check$/ }),
  );
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await act(async () => {
    complete!({ url: "https://google.com" });
  });
  await user.click(
    screen.getByRole("button", { name: /验证代理|Check connection/ }),
  );
  expect(
    within(await screen.findByRole("dialog")).getByLabelText(
      /探测网址|URL to check/,
    ),
  ).toHaveValue("https://new.example/check");
});
