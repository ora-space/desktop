import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import type { CloudConnection, ContractsClient } from "@ora/contracts";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { CloudConnectionSettings } from "./cloud-connection-settings";

void appI18n;

const SERVER = "https://cloud.example";
const alice = { id: "u-alice", displayName: "Alice" };

/** A host whose Cloud connection the test sets and inspects; it mirrors the host's transitions. */
function createHost(initial: CloudConnection) {
  const host = { connection: initial, servers: [] as Array<string | null> };
  const handlers: TestHandlers = {
    getCloudConnection: async () => host.connection,
    setCloudServer: async ({ server }) => {
      host.servers.push(server);
      // A token held for the previous server is revoked there; this host cannot reach it.
      const previous = host.connection;
      const held =
        previous.kind === "configured" &&
        (previous.session.kind === "signedIn" ||
          previous.session.kind === "unreachable");
      host.connection =
        server === null
          ? { kind: "notConfigured" }
          : {
              kind: "configured",
              server: server.toLowerCase(),
              session: { kind: "signedOut", failure: null },
            };
      return {
        connection: host.connection,
        revocation: held ? "unconfirmed" : null,
      };
    },
    startCloudLogin: async () => {
      host.connection = {
        kind: "configured",
        server: SERVER,
        session: { kind: "signingIn" },
      };
      return host.connection;
    },
    cancelCloudLogin: async () => {
      host.connection = {
        kind: "configured",
        server: SERVER,
        session: { kind: "signedOut", failure: null },
      };
      return host.connection;
    },
    logoutCloud: async () => {
      host.connection = {
        kind: "configured",
        server: SERVER,
        session: { kind: "signedOut", failure: null },
      };
      return { connection: host.connection, revocation: "unconfirmed" };
    },
  };
  return { host, client: createTestClient(handlers) };
}

/** A platform whose Cloud change events the test can emit. */
function platformWithEvents() {
  let listener: ((connection: CloudConnection) => void) | undefined;
  const stop = vi.fn();
  const platform: PlatformAdapter = {
    ...createStubPlatform(),
    cloudConnection: {
      onChanged: vi.fn(async (next) => {
        listener = next;
        return stop;
      }),
    },
  };
  return {
    platform,
    stop,
    emit: (connection: CloudConnection) => listener?.(connection),
  };
}

/** Renders the panel with an isolated query cache, contracts client and platform. */
async function renderPanel(client: ContractsClient, platform: PlatformAdapter) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <PlatformProvider adapter={platform}>
        <ContractsClientContext.Provider value={client}>
          <AppI18nProvider>
            <CloudConnectionSettings />
          </AppI18nProvider>
        </ContractsClientContext.Provider>
      </PlatformProvider>
    </QueryClientProvider>,
  );
  await screen.findByLabelText(/服务器地址|Server address/);
  return view;
}

it("saves a server and starts the browser sign-in, then shows the host's result", async () => {
  const { host, client } = createHost({ kind: "notConfigured" });
  const { platform, emit } = platformWithEvents();
  const user = userEvent.setup();
  await renderPanel(client, platform);

  expect(screen.getByRole("status")).toHaveTextContent(
    /尚未设置服务器|No server is set/,
  );
  await user.type(
    screen.getByLabelText(/服务器地址|Server address/),
    " https://Cloud.Example ",
  );
  await user.click(screen.getByRole("button", { name: /^(保存|Save)$/ }));
  await screen.findByRole("button", { name: /^(登录|Sign in)$/ });
  expect(host.servers).toEqual(["https://Cloud.Example"]);

  await user.click(screen.getByRole("button", { name: /^(登录|Sign in)$/ }));
  await screen.findByRole("button", { name: /取消登录|Cancel sign-in/ });

  // The sign-in finishes in the browser; the host announces it.
  act(() =>
    emit({
      kind: "configured",
      server: SERVER,
      session: { kind: "signedIn", account: alice },
    }),
  );
  expect(
    await screen.findByText(/已登录为 Alice|Signed in as Alice/),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: /退出登录|Sign out/ }),
  ).toBeEnabled();
});

it("explains a revoked sign-in and an unconfirmed sign-out", async () => {
  const { client } = createHost({
    kind: "configured",
    server: SERVER,
    session: { kind: "unreachable", account: alice },
  });
  const { platform, emit } = platformWithEvents();
  const user = userEvent.setup();
  await renderPanel(client, platform);

  expect(
    screen.getByText(/服务器暂时连不上|cannot be reached/),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /退出登录|Sign out/ }));
  expect(
    await screen.findByText(
      /没有确认登录已吊销|could not be reached to confirm/,
    ),
  ).toBeInTheDocument();

  act(() =>
    emit({
      kind: "configured",
      server: SERVER,
      session: { kind: "needsSignIn" },
    }),
  );
  expect(await screen.findByText(/登录已失效|has expired/)).toBeInTheDocument();
});

it("says why Cloud is off when the credential store is unavailable", async () => {
  const { client } = createHost({
    kind: "configured",
    server: SERVER,
    session: { kind: "storageUnavailable", reason: "no secret service" },
  });
  const { platform } = platformWithEvents();
  await renderPanel(client, platform);
  expect(screen.getByRole("status")).toHaveTextContent(/no secret service/);
  expect(
    screen.getByRole("button", { name: /^(登录|Sign in)$/ }),
  ).toBeEnabled();
});

it("cancels a sign-in in progress and stops listening when unmounted", async () => {
  const { host, client } = createHost({
    kind: "configured",
    server: SERVER,
    session: { kind: "signingIn" },
  });
  const { platform, stop } = platformWithEvents();
  const user = userEvent.setup();
  const view = await renderPanel(client, platform);
  await user.click(
    screen.getByRole("button", { name: /取消登录|Cancel sign-in/ }),
  );
  await waitFor(() =>
    expect(host.connection).toEqual({
      kind: "configured",
      server: SERVER,
      session: { kind: "signedOut", failure: null },
    }),
  );
  expect(
    await screen.findByText(/^(未登录。|Not signed in\.)$/),
  ).toBeInTheDocument();
  view.unmount();
  expect(stop).toHaveBeenCalled();
});

it("says the previous server did not confirm when changing the address signs out", async () => {
  const { host, client } = createHost({
    kind: "configured",
    server: SERVER,
    session: { kind: "signedIn", account: alice },
  });
  const { platform } = platformWithEvents();
  const user = userEvent.setup();
  await renderPanel(client, platform);

  const input = screen.getByLabelText(/服务器地址|Server address/);
  await user.clear(input);
  await user.type(input, "https://other.example");
  await user.click(screen.getByRole("button", { name: /^(保存|Save)$/ }));
  expect(
    await screen.findByText(
      /没有确认登录已吊销|could not be reached to confirm/,
    ),
  ).toBeInTheDocument();
  expect(host.servers).toEqual(["https://other.example"]);

  // Forgetting an address that held no token has nothing to revoke.
  await user.clear(screen.getByLabelText(/服务器地址|Server address/));
  await user.click(screen.getByRole("button", { name: /^(保存|Save)$/ }));
  expect(
    await screen.findByText(/尚未设置服务器|No server is set/),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/没有确认登录已吊销|could not be reached to confirm/),
  ).not.toBeInTheDocument();
});
