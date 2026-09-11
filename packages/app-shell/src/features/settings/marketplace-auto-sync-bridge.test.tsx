import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import {
  PlatformProvider,
  type MarketplaceAutoSyncEvent,
  type PlatformAdapter,
} from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import { pluginKeys } from "../../state/data/plugins";
import { useMarketplaceSyncStore } from "../../state/stores/marketplace-sync-store";
import { MarketplaceAutoSyncBridge } from "./marketplace-auto-sync-bridge";

afterEach(() => {
  act(() =>
    useMarketplaceSyncStore.setState({
      hostRefreshing: false,
      userSyncing: false,
    }),
  );
});

/** Builds a platform whose host refreshes can be driven from the test. */
function platformWithAutoSync(stop: () => void) {
  let report: ((event: MarketplaceAutoSyncEvent) => void) | undefined;
  const platform: PlatformAdapter = {
    ...createStubPlatform(),
    pluginMarketplace: {
      onInstallProgress: vi.fn(async () => () => undefined),
      onAutoSyncChanged: vi.fn(async (listener) => {
        report = listener;
        return stop;
      }),
    },
  };
  return {
    platform,
    emit: (event: MarketplaceAutoSyncEvent) => report?.(event),
  };
}

/** Renders the bridge over an isolated query cache so invalidation can be observed. */
async function renderBridge(platform: PlatformAdapter) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <PlatformProvider adapter={platform}>
        <MarketplaceAutoSyncBridge />
      </PlatformProvider>
    </QueryClientProvider>,
  );
  await act(async () => Promise.resolve());
  return { view, queryClient };
}

/** The Sync action reads this state to stand down for a refresh the host started itself. */
it("tracks the span of a host-initiated refresh", async () => {
  const { platform, emit } = platformWithAutoSync(() => undefined);
  await renderBridge(platform);

  act(() => emit({ kind: "started" }));
  expect(useMarketplaceSyncStore.getState().hostRefreshing).toBe(true);

  act(() => emit({ kind: "finished" }));
  expect(useMarketplaceSyncStore.getState().hostRefreshing).toBe(false);
});

/** A refresh that rebuilt the index must reach the cached listing, page open or not. */
it("invalidates the cached listing when a refresh finishes", async () => {
  const { platform, emit } = platformWithAutoSync(() => undefined);
  const { queryClient } = await renderBridge(platform);
  const invalidate = vi.spyOn(queryClient, "invalidateQueries");

  act(() => emit({ kind: "started" }));
  expect(invalidate).not.toHaveBeenCalled();

  await act(async () => {
    emit({ kind: "finished" });
    await Promise.resolve();
  });

  expect(invalidate).toHaveBeenCalledWith({
    queryKey: pluginKeys.availablePlugins,
  });
});

/** Unmounting must not strand the Sync action behind a `finished` that can no longer arrive. */
it("releases the running state and its subscription on unmount", async () => {
  const stop = vi.fn();
  const { platform, emit } = platformWithAutoSync(stop);
  const { view } = await renderBridge(platform);

  act(() => emit({ kind: "started" }));
  view.unmount();

  expect(stop).toHaveBeenCalledOnce();
  expect(useMarketplaceSyncStore.getState().hostRefreshing).toBe(false);
});
