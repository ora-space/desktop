import { act, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { GetEffectTargetStatusRequest } from "@ora/contracts";
import { createTestClient } from "../../test/contracts-transport";
import { renderHookWithClient } from "../../test/hook-harness";
import { AGENT_REF } from "../../test/agent-identity";
import { readyEffectHandlers } from "../../test/memory/effects";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import {
  effectTargetFailureMessage,
  effectTargetStatusPollInterval,
  useAgentEffectReadiness,
} from "./use-agent-effect-readiness";

/** Verifies that an installed agent addresses its own canonical Consumer Target. */
async function readinessFor(agentRef: string) {
  const getEffectTargetStatus = vi.fn(
    async (request: GetEffectTargetStatusRequest) => {
      void request;
      return { status: null };
    },
  );
  const plugins = createPluginMemory();
  const { result } = renderHookWithClient(
    () => useAgentEffectReadiness("workspace-1", agentRef),
    createTestClient({ ...pluginHandlers(plugins), getEffectTargetStatus }),
  );
  await waitFor(() => expect(getEffectTargetStatus).toHaveBeenCalledOnce());
  expect(result.current).toEqual({ kind: "blocked" });
  return getEffectTargetStatus;
}

describe("useAgentEffectReadiness", () => {
  it.each([AGENT_REF.opencode, AGENT_REF.nga, AGENT_REF.codeagentcli])(
    "waits for installed agent %s using its canonical plugin identity",
    async (agentRef) => {
      const getEffectTargetStatus = await readinessFor(agentRef);

      expect(getEffectTargetStatus).toHaveBeenCalledOnce();
      expect(getEffectTargetStatus.mock.calls[0]![0]).toEqual({
        selector: "workspace_agent",
        workspaceId: "workspace-1",
        agentPluginId: agentRef,
      });
    },
  );

  it("gates an installed third-party agent without adding it to a frontend allowlist", async () => {
    const agentRef = "community/example-agent";
    const getEffectTargetStatus = vi.fn(async () => ({ status: null }));
    const plugins = createPluginMemory();
    const installedAgent = plugins.installedPlugins[0]!;
    plugins.installedPlugins = [
      {
        ...installedAgent,
        id: agentRef,
        namespace: "community",
        name: "example-agent",
      },
    ];
    const { result } = renderHookWithClient(
      () => useAgentEffectReadiness("workspace-1", agentRef),
      createTestClient({ ...pluginHandlers(plugins), getEffectTargetStatus }),
    );

    await waitFor(() => expect(getEffectTargetStatus).toHaveBeenCalledOnce());
    expect(result.current).toEqual({ kind: "blocked" });
  });

  it("does not gate an agent missing from the local installed-plugin list", async () => {
    const getEffectTargetStatus = vi.fn();
    const plugins = createPluginMemory();
    const { result } = renderHookWithClient(
      () => useAgentEffectReadiness("workspace-1", "community/example-agent"),
      createTestClient({ ...pluginHandlers(plugins), getEffectTargetStatus }),
    );

    await waitFor(() => expect(result.current).toEqual({ kind: "ready" }));
    expect(getEffectTargetStatus).not.toHaveBeenCalled();
  });

  it("blocks the first prompt while the Target status request is pending", async () => {
    const getEffectTargetStatus = vi.fn(() => new Promise<never>(() => {}));
    const plugins = createPluginMemory();
    const { result } = renderHookWithClient(
      () => useAgentEffectReadiness("workspace-1", AGENT_REF.claude),
      createTestClient({ ...pluginHandlers(plugins), getEffectTargetStatus }),
    );

    await act(async () => {});
    expect(result.current).toEqual({ kind: "blocked" });
  });

  it("stops polling after the Target is ready", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const getEffectTargetStatus = vi.fn(
      readyEffectHandlers().getEffectTargetStatus,
    );
    const plugins = createPluginMemory();
    const { result, unmount } = renderHookWithClient(
      () => useAgentEffectReadiness("workspace-1", AGENT_REF.claude),
      createTestClient({ ...pluginHandlers(plugins), getEffectTargetStatus }),
    );

    await waitFor(() => expect(result.current).toEqual({ kind: "ready" }));
    expect(getEffectTargetStatus).toHaveBeenCalledOnce();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_000);
    });
    expect(getEffectTargetStatus).toHaveBeenCalledOnce();
    unmount();
    vi.useRealTimers();
  });

  it("polls every second while syncing, stops when ready, and backs off when failed", async () => {
    expect(effectTargetStatusPollInterval(undefined)).toBe(1_000);
    expect(effectTargetStatusPollInterval(null)).toBe(1_000);
    const ready = await readyEffectHandlers().getEffectTargetStatus();
    expect(effectTargetStatusPollInterval(ready.status)).toBe(false);

    const recovering = {
      ...ready.status!,
      phase: "recovery_required" as const,
    };
    expect(effectTargetStatusPollInterval(recovering)).toBe(10_000);
    expect(effectTargetFailureMessage(recovering)).toBe(
      "Effect recovery is required.",
    );
  });

  it("reports the manual blocking Condition as a failed gate with its reason", async () => {
    const ready = await readyEffectHandlers().getEffectTargetStatus();
    const drifted = {
      ...ready.status!,
      phase: "pending" as const,
      conditions: [
        {
          id: "condition-1",
          ownerKind: "resource",
          ownerId: "resource-1",
          subjectKind: "managed_item",
          subjectId: "managed-1",
          code: "managed_item_drift",
          impact: "blocking" as const,
          retry: "manual" as const,
          generation: 1n,
          message: "A Managed Item changed outside Ora.",
          firstObservedAt: 1n,
          lastObservedAt: 1n,
        },
      ],
    };
    expect(effectTargetFailureMessage(drifted)).toBe(
      "A Managed Item changed outside Ora.",
    );
    expect(effectTargetStatusPollInterval(drifted)).toBe(10_000);

    // A blocking Condition that can still resolve on its own stays a sync wait.
    const waiting = {
      ...drifted,
      conditions: [{ ...drifted.conditions[0]!, retry: "on_change" as const }],
    };
    expect(effectTargetFailureMessage(waiting)).toBeNull();
    expect(effectTargetStatusPollInterval(waiting)).toBe(1_000);
  });

  it("marks a Target in recovery as failed", async () => {
    const status = await readyEffectHandlers().getEffectTargetStatus();
    const getEffectTargetStatus = vi.fn(async () => ({
      status: {
        ...status.status!,
        phase: "recovery_required" as const,
        recoveryOperationId: "operation-1",
      },
    }));
    const plugins = createPluginMemory();
    const { result } = renderHookWithClient(
      () => useAgentEffectReadiness("workspace-1", AGENT_REF.claude),
      createTestClient({ ...pluginHandlers(plugins), getEffectTargetStatus }),
    );

    await waitFor(() =>
      expect(result.current).toEqual({
        kind: "failed",
        message: "Effect recovery is required.",
      }),
    );
  });
});
