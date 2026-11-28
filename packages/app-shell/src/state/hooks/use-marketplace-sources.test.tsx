import { act, waitFor } from "@testing-library/react";
import type { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import "../../i18n/i18n-instance";
import { pluginKeys } from "../data/plugins";
import { skillKeys } from "../data/skills";
import {
  useAddMarketplaceSource,
  useUpdateMarketplaceSource,
} from "./use-marketplace-sources";

const S3_RETRIEVAL = {
  type: "s3_sigv4",
  endpoint: "https://s3.example.com",
  bucket: "plugins",
  region: "region-1",
  credentials: {
    action: "replace",
    accessKeyId: "access",
    secretAccessKey: "secret",
  },
} as const;

/** Seeds the four origin-bearing caches so a prefix invalidation is observable. */
function seedOriginCaches(queryClient: QueryClient): void {
  queryClient.setQueryData(pluginKeys.marketplaceSources, { sources: [] });
  queryClient.setQueryData(pluginKeys.installedPlugins, { plugins: [] });
  queryClient.setQueryData(pluginKeys.availablePlugins, {
    plugins: [],
    updatedAt: 0n,
    failedSources: [],
  });
  queryClient.setQueryData(skillKeys.skills, { skills: [] });
}

/** Reports whether each origin-bearing cache was invalidated. */
function invalidated(queryClient: QueryClient) {
  return {
    sources:
      queryClient.getQueryState(pluginKeys.marketplaceSources)?.isInvalidated,
    installed:
      queryClient.getQueryState(pluginKeys.installedPlugins)?.isInvalidated,
    available:
      queryClient.getQueryState(pluginKeys.availablePlugins)?.isInvalidated,
    skills: queryClient.getQueryState(skillKeys.skills)?.isInvalidated,
  };
}

const ALL_INVALIDATED = {
  sources: true,
  installed: true,
  available: true,
  skills: true,
};

describe("marketplace source mutations refresh origins even when they fail", () => {
  it("invalidates origin queries when the add's credential step fails", async () => {
    const handlers: TestHandlers = pluginHandlers(createPluginMemory());
    const client = createTestClient({
      ...handlers,
      // The add stores Direct HTTPS first; the SigV4 credential write is the second step.
      updateMarketplaceSource: async () => {
        throw new Error("credential write failed");
      },
    });
    const queryClient = createTestQueryClient();
    seedOriginCaches(queryClient);
    const { result } = renderHookWithClient(
      () => useAddMarketplaceSource(),
      client,
      queryClient,
    );

    await act(async () => {
      await expect(
        result.current.mutateAsync({
          url: "https://github.com/acme/marketplace",
          branch: "main",
          useProxy: false,
          artifactRetrieval: S3_RETRIEVAL,
        }),
      ).rejects.toThrow("credential write failed");
    });

    // A failed add still changed bindings (and rolled a row back), so the labels must refresh.
    await waitFor(() =>
      expect(invalidated(queryClient)).toEqual(ALL_INVALIDATED),
    );
  });

  it("invalidates origin queries when an update fails", async () => {
    const state = createPluginMemory();
    state.marketplaceSources.push({
      url: "https://github.com/acme/marketplace",
      branch: "main",
      useProxy: false,
      enabled: true,
      artifactRetrieval: { type: "direct_https" },
    });
    const handlers: TestHandlers = pluginHandlers(state);
    const client = createTestClient({
      ...handlers,
      updateMarketplaceSource: async () => {
        throw new Error("save rejected");
      },
    });
    const queryClient = createTestQueryClient();
    seedOriginCaches(queryClient);
    const { result } = renderHookWithClient(
      () => useUpdateMarketplaceSource(),
      client,
      queryClient,
    );

    await act(async () => {
      await expect(
        result.current.mutateAsync({
          url: "https://github.com/acme/marketplace",
          newUrl: "github.com/acme/marketplace",
          branch: "main",
          useProxy: false,
          enabled: true,
          artifactRetrieval: { type: "direct_https" },
        }),
      ).rejects.toThrow("save rejected");
    });

    // A rejected edit must still refresh: the host may have bound a URL before validating it.
    await waitFor(() =>
      expect(invalidated(queryClient)).toEqual(ALL_INVALIDATED),
    );
  });
});