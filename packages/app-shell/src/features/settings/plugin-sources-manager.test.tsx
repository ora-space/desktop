import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import type { ContractsClient } from "@ora/contracts";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { PluginSourcesManager } from "./plugin-sources-manager";

// Keep this test worker responsible for initializing the instance used by useTranslation.
void appI18n;

/** Renders the source manager with an isolated query client and contracts client. */
function renderManager(client: ContractsClient, onBack = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>
          <PluginSourcesManager onBack={onBack} />
        </AppI18nProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
}

it("renders configured marketplace sources", async () => {
  const state = createMockClientState();
  state.marketplaceSources.push({
    url: "https://github.com/ora-space/marketplace",
    branch: "main",
    useProxy: false,
  });
  const client = createMockClient(state);

  renderManager(client);

  expect(
    await screen.findByText("https://github.com/ora-space/marketplace"),
  ).toBeInTheDocument();
});

it("adds a marketplace source through the backend", async () => {
  const state = createMockClientState();
  const client = createMockClient(state);
  const user = userEvent.setup();

  renderManager(client);

  await user.type(
    screen.getByLabelText(/Git URL/),
    "https://github.com/example/marketplace",
  );
  await user.click(screen.getByRole("button", { name: /添加来源|Add source/ }));

  await waitFor(() =>
    expect(state.marketplaceSources).toEqual([
      {
        url: "https://github.com/example/marketplace",
        branch: "main",
        useProxy: false,
      },
    ]),
  );
});

it("removes a marketplace source through the backend", async () => {
  const state = createMockClientState();
  state.marketplaceSources.push({
    url: "https://github.com/ora-space/marketplace",
    branch: "main",
    useProxy: false,
  });
  const client = createMockClient(state);
  const deleteSource = vi.spyOn(client.plugin, "deleteSource");

  renderManager(client);

  const deleteButton = await screen.findByRole("button", {
    name: /删除|Delete/,
  });
  expect(deleteButton).toBeEnabled();
  fireEvent.click(deleteButton);

  await waitFor(() =>
    expect(deleteSource).toHaveBeenCalledWith({
      url: "https://github.com/ora-space/marketplace",
    }),
  );
  await waitFor(() => expect(state.marketplaceSources).toEqual([]));
});
