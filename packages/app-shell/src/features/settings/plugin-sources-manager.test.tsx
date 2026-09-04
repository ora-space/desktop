import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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
    enabled: true,
    artifactRetrieval: { type: "direct_https" },
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
        enabled: true,
        artifactRetrieval: { type: "direct_https" },
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
    enabled: true,
    artifactRetrieval: { type: "direct_https" },
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

it("edits a marketplace source URL and branch", async () => {
  const state = createMockClientState();
  state.marketplaceSources.push({
    url: "https://github.com/ora-space/marketplace",
    branch: "main",
    useProxy: false,
    enabled: true,
    artifactRetrieval: { type: "direct_https" },
  });
  const client = createMockClient(state);
  const user = userEvent.setup();

  renderManager(client);

  await user.click(await screen.findByRole("button", { name: /编辑|Edit/ }));
  const dialog = await screen.findByRole("dialog");
  const urlInput = within(dialog).getByLabelText(/Git URL/);
  const branchInput = within(dialog).getByLabelText(/分支|Branch/);
  await user.clear(urlInput);
  await user.type(urlInput, "https://github.com/example/marketplace");
  await user.clear(branchInput);
  await user.type(branchInput, "release");
  await user.click(within(dialog).getByRole("button", { name: /保存|Save/ }));

  await waitFor(() =>
    expect(state.marketplaceSources).toEqual([
      {
        url: "https://github.com/example/marketplace",
        branch: "release",
        useProxy: false,
        enabled: true,
        artifactRetrieval: { type: "direct_https" },
      },
    ]),
  );
});

it("configures S3 SigV4 retrieval without returning credentials", async () => {
  const state = createMockClientState();
  state.marketplaceSources.push({
    url: "https://github.com/ora-space/marketplace",
    branch: "main",
    useProxy: false,
    enabled: true,
    artifactRetrieval: { type: "direct_https" },
  });
  const client = createMockClient(state);
  const updateSource = vi.spyOn(client.plugin, "updateSource");
  const user = userEvent.setup();

  renderManager(client);

  await user.click(await screen.findByRole("button", { name: /编辑|Edit/ }));
  const dialog = await screen.findByRole("dialog");
  await user.click(
    within(dialog).getByLabelText(/插件包获取方式|Plugin package retrieval/),
  );
  await user.click(await screen.findByRole("option", { name: /S3.*SigV4/i }));
  await user.type(
    within(dialog).getByLabelText(/S3 Endpoint/i),
    "https://s3.example.com",
  );
  await user.type(within(dialog).getByLabelText(/^Bucket$/i), "plugins");
  await user.type(within(dialog).getByLabelText(/^Region$/i), "region-1");
  await user.type(within(dialog).getByLabelText(/Access Key ID/i), "access");
  await user.type(
    within(dialog).getByLabelText(/Secret Access Key/i),
    "secret",
  );
  await user.click(within(dialog).getByRole("button", { name: /保存|Save/ }));

  await waitFor(() =>
    expect(updateSource).toHaveBeenCalledWith({
      url: "https://github.com/ora-space/marketplace",
      newUrl: "https://github.com/ora-space/marketplace",
      branch: "main",
      useProxy: false,
      enabled: true,
      artifactRetrieval: {
        type: "s3_sigv4",
        endpoint: "https://s3.example.com",
        bucket: "plugins",
        region: "region-1",
        credentials: {
          action: "replace",
          accessKeyId: "access",
          secretAccessKey: "secret",
        },
      },
    }),
  );
  await waitFor(() =>
    expect(state.marketplaceSources[0]?.artifactRetrieval).toEqual({
      type: "s3_sigv4",
      endpoint: "https://s3.example.com",
      bucket: "plugins",
      region: "region-1",
    }),
  );
});

it("disables a marketplace source without removing it", async () => {
  const state = createMockClientState();
  state.marketplaceSources.push({
    url: "https://github.com/ora-space/marketplace",
    branch: "main",
    useProxy: false,
    enabled: true,
    artifactRetrieval: { type: "direct_https" },
  });
  const client = createMockClient(state);
  const user = userEvent.setup();

  renderManager(client);

  await user.click(await screen.findByRole("button", { name: /禁用|Disable/ }));

  await waitFor(() =>
    expect(state.marketplaceSources).toEqual([
      {
        url: "https://github.com/ora-space/marketplace",
        branch: "main",
        useProxy: false,
        enabled: false,
        artifactRetrieval: { type: "direct_https" },
      },
    ]),
  );
});
