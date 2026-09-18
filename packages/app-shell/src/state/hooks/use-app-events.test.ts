import { waitFor } from "@testing-library/react";
import type { AppEvent } from "@ora/contracts";
import { describe, expect, it, vi } from "vitest";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import "../../i18n/i18n-instance";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { sessionKeys } from "../data/sessions";
import { pluginKeys } from "../data/plugins";
import { agentRuntimeKeys } from "../data/agent-runtime";
import { mcpHealthKeys } from "../data/mcp-health";
import { workflowRunKeys } from "../data/workflow-runs";
import { useAppEvents } from "./use-app-events";

describe("useAppEvents", () => {
  it("refetches after Ready and invalidates sessions for title events", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.watchAppEvents = async function* (
      _request,
      options,
    ): AsyncGenerator<AppEvent> {
      yield { type: "ready" };
      yield { type: "session_title_updated", session_id: "session-1" };
      await new Promise<void>((resolve) => {
        const signal = options?.signal;
        if (signal === undefined || signal.aborted) {
          resolve();
          return;
        }
        signal.addEventListener("abort", () => resolve(), { once: true });
      });
    };
    const queryClient = createTestQueryClient();
    const refetch = vi.spyOn(queryClient, "refetchQueries");
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");

    const { result, unmount } = renderHookWithClient(
      () => useAppEvents(client),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(refetch).toHaveBeenCalledWith({ queryKey: sessionKeys.sessions });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: sessionKeys.sessions });

    unmount();
  });

  it("invalidates the plugin snapshot and agent detection for lifecycle events", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.watchAppEvents = async function* (
      _request,
      options,
    ): AsyncGenerator<AppEvent> {
      yield { type: "ready" };
      yield { type: "plugin_status_changed", plugin_id: "official/example" };
      await new Promise<void>((resolve) => {
        const signal = options?.signal;
        if (signal === undefined || signal.aborted) {
          resolve();
          return;
        }
        signal.addEventListener("abort", () => resolve(), { once: true });
      });
    };
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");

    const { result, unmount } = renderHookWithClient(
      () => useAppEvents(client),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.ready).toBe(true));
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: pluginKeys.installedPlugins,
      }),
    );
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: agentRuntimeKeys.agentRuntimeStatus,
    });

    unmount();
  });

  it("re-queries the run detail and lists for run invalidation events", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.watchAppEvents = async function* (
      _request,
      options,
    ): AsyncGenerator<AppEvent> {
      yield { type: "ready" };
      yield { type: "workflow_run_invalidated", run_id: "run-1" };
      await new Promise<void>((resolve) => {
        const signal = options?.signal;
        if (signal === undefined || signal.aborted) {
          resolve();
          return;
        }
        signal.addEventListener("abort", () => resolve(), { once: true });
      });
    };
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");

    const { result, unmount } = renderHookWithClient(
      () => useAppEvents(client),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.ready).toBe(true));
    // The event carries only the run id; the reaction is a re-query of the authoritative run
    // detail plus the run lists, never a write of event payload into query state.
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: workflowRunKeys.detail("run-1"),
      }),
    );
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: workflowRunKeys.projectLists,
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: workflowRunKeys.workflowLists,
    });

    unmount();
  });

  it("re-queries Host MCP health for a health change event", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.watchAppEvents = async function* (
      _request,
      options,
    ): AsyncGenerator<AppEvent> {
      yield { type: "ready" };
      yield { type: "mcp_health_changed", plugin_id: "official/tavily" };
      await new Promise<void>((resolve) => {
        const signal = options?.signal;
        if (signal === undefined || signal.aborted) {
          resolve();
          return;
        }
        signal.addEventListener("abort", () => resolve(), { once: true });
      });
    };
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");

    const { result, unmount } = renderHookWithClient(
      () => useAppEvents(client),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.ready).toBe(true));
    // The event carries identity only; the health views are re-queried rather than written from
    // the event, so a lost event can never leave a fabricated result behind.
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({ queryKey: mcpHealthKeys.all }),
    );

    unmount();
  });
});
