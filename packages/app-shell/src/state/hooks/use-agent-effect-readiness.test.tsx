import { act, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { GetEffectTargetStatusRequest } from "@ora/contracts";
import { createTestClient } from "../../test/contracts-transport";
import { renderHookWithClient } from "../../test/hook-harness";
import { AGENT_REF } from "../../test/agent-identity";
import { readyEffectHandlers } from "../../test/memory/effects";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import {
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
  expect(result.current).toBe("blocked");
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
    expect(result.current).toBe("blocked");
  });

  it("does not gate an agent missing from the local installed-plugin list", async () => {
    const getEffectTargetStatus = vi.fn();
    const plugins = createPluginMemory();
    const { result } = renderHookWithClient(
      () => useAgentEffectReadiness("workspace-1", "community/example-agent"),
      createTestClient({ ...pluginHandlers(plugins), getEffectTargetStatus }),
    );

    await waitFor(() => expect(result.current).toBe("ready"));
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
    expect(result.current).toBe("blocked");
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

    await waitFor(() => expect(result.current).toBe("ready"));
    expect(getEffectTargetStatus).toHaveBeenCalledOnce();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_000);
    });
    expect(getEffectTargetStatus).toHaveBeenCalledOnce();
    unmount();
    vi.useRealTimers();
  });

  it("polls every second until Target status is ready, then stops", async () => {
    expect(effectTargetStatusPollInterval(undefined)).toBe(1_000);
    expect(effectTargetStatusPollInterval(null)).toBe(1_000);
    const ready = await readyEffectHandlers().getEffectTargetStatus({
      selector: "workspace_agent",
      workspaceId: "workspace-1",
      agentPluginId: AGENT_REF.claude,
    });
    expect(effectTargetStatusPollInterval(ready.status)).toBe(false);
  });
});
