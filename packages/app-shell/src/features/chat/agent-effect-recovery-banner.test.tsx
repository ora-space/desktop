import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement } from "react";
import type { ContractsClient } from "@ora/contracts";
import { QueryClient } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createTestClient } from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createChatStore } from "@ora/chat";
import { appI18n } from "../../i18n/i18n-instance";
import type { RecoverEffectTargetResponse } from "@ora/contracts";
import type { AgentEffectReadiness } from "../../state/hooks/use-agent-effect-readiness";
import {
  localizeAgentEffectFailureReason,
  localizeAgentEffectFailureSummary,
} from "./agent-effect-failure-reason";
import { AgentEffectRecoveryBanner } from "./agent-effect-recovery-banner";
import { useAgentEffectReadiness } from "../../state/hooks/use-agent-effect-readiness";
import { AGENT_REF } from "../../test/agent-identity";
import { readyEffectHandlers } from "../../test/memory/effects";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import type { GetEffectTargetStatusRequest } from "@ora/contracts";

/** The failed readiness a quarantined Effect Target produces. */
const quarantined: AgentEffectReadiness = {
  kind: "failed",
  message: "An Effect operation requires explicit recovery.",
  reasons: [
    {
      code: "recovery_required",
      message: "An Effect operation requires explicit recovery.",
      parameters: {},
    },
  ],
  targetId: "target-1",
  recoveryRequired: true,
};

/** Mounts the banner with its providers but without any workspace-specific state. */
function renderBanner(
  readiness: AgentEffectReadiness,
  client: ContractsClient,
  queryClient?: QueryClient,
) {
  const queries = queryClient ?? createTestQueryClient();
  const wrapper = createHookWrapper(
    client,
    queries,
    createChatStore(client.session),
  );
  return {
    ...render(createElement(AgentEffectRecoveryBanner, { readiness }), {
      wrapper,
    }),
    queries,
  };
}

describe("AgentEffectRecoveryBanner", () => {
  afterEach(async () => {
    await act(async () => {
      await appI18n.changeLanguage("zh-CN");
    });
  });

  it("refreshes real readiness after recovery and isolates other workspaces and agents", async () => {
    const ready = await readyEffectHandlers().getEffectTargetStatus();
    let recovered = false;
    const getEffectTargetStatus = vi.fn(
      async (request: GetEffectTargetStatusRequest) => {
        if (request.selector !== "workspace_agent")
          throw new Error("expected workspace selector");
        const selected =
          request.workspaceId === "workspace-1" &&
          request.agentPluginId === AGENT_REF.claude;
        return {
          status: {
            ...ready.status,
            targetId: selected
              ? "target-1"
              : `${request.workspaceId}/${request.agentPluginId}`,
            phase:
              selected && !recovered
                ? ("recovery_required" as const)
                : ("current" as const),
          },
        };
      },
    );
    const recoverEffectTarget = vi.fn(async (request: { targetId: string }) => {
      expect(request).toEqual({ targetId: "target-1" });
      recovered = true;
      return {
        recovered: true,
        finalizedOperations: 1,
        closedWithoutEffectOperations: 0,
      };
    });
    const client = createTestClient({
      ...pluginHandlers(createPluginMemory()),
      getEffectTargetStatus,
      recoverEffectTarget,
    });
    function Gate({
      workspaceId,
      agentRef,
    }: {
      workspaceId: string;
      agentRef: string;
    }) {
      const readiness = useAgentEffectReadiness(workspaceId, agentRef);
      return (
        <>
          <button
            aria-label={`send ${workspaceId} ${agentRef}`}
            disabled={readiness.kind !== "ready"}
          >
            Send
          </button>
          <AgentEffectRecoveryBanner readiness={readiness} />
        </>
      );
    }
    render(
      <>
        <Gate workspaceId="workspace-1" agentRef={AGENT_REF.claude} />
        <Gate workspaceId="workspace-2" agentRef={AGENT_REF.claude} />
        <Gate workspaceId="workspace-1" agentRef={AGENT_REF.opencode} />
      </>,
      {
        wrapper: createHookWrapper(
          client,
          createTestQueryClient(),
          createChatStore(client.session),
        ),
      },
    );
    const send = screen.getByRole("button", {
      name: `send workspace-1 ${AGENT_REF.claude}`,
    });
    await screen.findByRole("alert");
    await waitFor(() => expect(getEffectTargetStatus).toHaveBeenCalledTimes(3));
    expect(send).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: /恢复|Recover/ }));
    await waitFor(() => expect(send).toBeEnabled());
    expect(recoverEffectTarget).toHaveBeenCalledOnce();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      getEffectTargetStatus.mock.calls.map(([request]) => request),
    ).toEqual([
      {
        selector: "workspace_agent",
        workspaceId: "workspace-1",
        agentPluginId: AGENT_REF.claude,
      },
      {
        selector: "workspace_agent",
        workspaceId: "workspace-2",
        agentPluginId: AGENT_REF.claude,
      },
      {
        selector: "workspace_agent",
        workspaceId: "workspace-1",
        agentPluginId: AGENT_REF.opencode,
      },
      {
        selector: "workspace_agent",
        workspaceId: "workspace-1",
        agentPluginId: AGENT_REF.claude,
      },
    ]);
  });

  it.each([
    {
      code: "preserved_item_conflict",
      message:
        "Move the unmanaged directory that occupies the skill directory: .agents/skills/grilling",
      parameters: { relativePath: ".agents/skills/grilling" },
      expectedZh:
        "存在未由 Ora 管理的旧技能目录，无法覆盖：.agents/skills/grilling",
      hint: /Move the old Skill directories|\u79fb\u8d70/,
    },
    {
      code: "skill_directory_link",
      message:
        "Symbolic links and Windows junctions are not supported: .agents/skills/review/vendor/linked",
      parameters: { relativePath: ".agents/skills/review/vendor/linked" },
      expectedZh:
        "技能目录不支持符号链接或 Windows junction：.agents/skills/review/vendor/linked",
      hint: /Replace the listed symbolic links|\u66ff\u6362\u4e3a\u666e\u901a/,
    },
  ])(
    "shows $code and retries only the affected Target",
    async ({ code, message, parameters, expectedZh, hint }) => {
      await act(async () => {
        await appI18n.changeLanguage("zh-CN");
      });
      const ready = await readyEffectHandlers().getEffectTargetStatus();
      let retried = false;
      const getEffectTargetStatus = vi.fn(async () => ({
        status: {
          ...ready.status!,
          targetId: "target-1",
          desiredGeneration: 1,
          observedGeneration: 1,
          appliedGeneration: retried ? 1 : 0,
          readyGeneration: retried ? 1 : 0,
          phase: retried ? ("current" as const) : ("planning" as const),
          conditions: retried
            ? []
            : [
                {
                  id: "condition-1",
                  ownerKind: "resource",
                  ownerId: "resource-1",
                  subjectKind: "desired_effect",
                  subjectId: "desired-1",
                  code,
                  impact: "blocking" as const,
                  retry: "on_change" as const,
                  generation: 1n,
                  message,
                  parameters,
                  firstObservedAt: 1n,
                  lastObservedAt: 1n,
                },
              ],
        },
      }));
      const retryEffectTarget = vi.fn(async (request: { targetId: string }) => {
        expect(request).toEqual({ targetId: "target-1" });
        retried = true;
        return { requested: true };
      });
      const client = createTestClient({
        ...pluginHandlers(createPluginMemory()),
        getEffectTargetStatus,
        retryEffectTarget,
      });
      function Gate() {
        const readiness = useAgentEffectReadiness(
          "workspace-1",
          AGENT_REF.claude,
        );
        return (
          <>
            <button aria-label="send" disabled={readiness.kind !== "ready"}>
              Send
            </button>
            <AgentEffectRecoveryBanner readiness={readiness} />
          </>
        );
      }
      render(<Gate />, {
        wrapper: createHookWrapper(
          client,
          createTestQueryClient(),
          createChatStore(client.session),
        ),
      });

      const banner = await screen.findByRole("alert");
      expect(banner).toHaveTextContent(expectedZh);
      expect(banner).not.toHaveTextContent(message);
      expect(banner).toHaveTextContent(hint);
      expect(screen.getByRole("button", { name: "send" })).toBeDisabled();
      await userEvent.click(
        screen.getByRole("button", { name: /重新同步|Retry sync/ }),
      );

      await waitFor(() =>
        expect(screen.getByRole("button", { name: "send" })).toBeEnabled(),
      );
      expect(retryEffectTarget).toHaveBeenCalledOnce();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    },
  );

  it("localizes same-name preserved conflicts across two roots in Chinese and English", async () => {
    await act(async () => {
      await appI18n.changeLanguage("zh-CN");
    });
    const ready = await readyEffectHandlers().getEffectTargetStatus();
    let retried = false;
    const getEffectTargetStatus = vi.fn(async () => ({
      status: {
        ...ready.status!,
        targetId: "target-1",
        desiredGeneration: 1,
        observedGeneration: 1,
        appliedGeneration: retried ? 1 : 0,
        readyGeneration: retried ? 1 : 0,
        phase: retried ? ("current" as const) : ("planning" as const),
        conditions: retried
          ? []
          : [
              {
                id: "condition-agents",
                ownerKind: "resource",
                ownerId: "resource-agents",
                subjectKind: "desired_effect",
                subjectId: "desired-1",
                code: "preserved_item_conflict",
                impact: "blocking" as const,
                retry: "on_change" as const,
                generation: 1n,
                message:
                  "Move the unmanaged directory that occupies the skill directory: .agents/skills/foo",
                parameters: { relativePath: ".agents/skills/foo" },
                firstObservedAt: 1n,
                lastObservedAt: 1n,
              },
              {
                id: "condition-opencode",
                ownerKind: "resource",
                ownerId: "resource-opencode",
                subjectKind: "desired_effect",
                subjectId: "desired-1",
                code: "preserved_item_conflict",
                impact: "blocking" as const,
                retry: "on_change" as const,
                generation: 1n,
                message:
                  "Move the unmanaged directory that occupies the skill directory: .opencode/skills/foo",
                parameters: { relativePath: ".opencode/skills/foo" },
                firstObservedAt: 1n,
                lastObservedAt: 1n,
              },
            ],
      },
    }));
    const retryEffectTarget = vi.fn(async (request: { targetId: string }) => {
      expect(request).toEqual({ targetId: "target-1" });
      retried = true;
      return { requested: true };
    });
    const client = createTestClient({
      ...pluginHandlers(createPluginMemory()),
      getEffectTargetStatus,
      retryEffectTarget,
    });
    function Gate() {
      const readiness = useAgentEffectReadiness(
        "workspace-1",
        AGENT_REF.opencode,
      );
      return (
        <>
          <button aria-label="send" disabled={readiness.kind !== "ready"}>
            Send
          </button>
          <AgentEffectRecoveryBanner readiness={readiness} />
        </>
      );
    }
    render(<Gate />, {
      wrapper: createHookWrapper(
        client,
        createTestQueryClient(),
        createChatStore(client.session),
      ),
    });

    const banner = await screen.findByRole("alert");
    const zhItems = screen
      .getAllByRole("listitem")
      .map((item) => item.textContent);
    expect(zhItems).toEqual([
      "存在未由 Ora 管理的旧技能目录，无法覆盖：.agents/skills/foo",
      "存在未由 Ora 管理的旧技能目录，无法覆盖：.opencode/skills/foo",
    ]);
    expect(banner).not.toHaveTextContent("Move the unmanaged directory");
    expect(banner).toHaveTextContent("请移走上列旧技能目录，然后重新同步。");

    await act(async () => {
      await appI18n.changeLanguage("en-US");
    });
    const enItems = screen
      .getAllByRole("listitem")
      .map((item) => item.textContent);
    expect(enItems).toEqual([
      "Unmanaged Skill directory cannot be overwritten: .agents/skills/foo",
      "Unmanaged Skill directory cannot be overwritten: .opencode/skills/foo",
    ]);
    expect(banner).toHaveTextContent(
      "Move the old Skill directories listed above out of the workspace, then retry the sync.",
    );

    await userEvent.click(screen.getByRole("button", { name: "Retry sync" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "send" })).toBeEnabled(),
    );
    expect(retryEffectTarget).toHaveBeenCalledOnce();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("localizes known codes and falls back to safe messages when parameters or codes are unknown", async () => {
    await act(async () => {
      await appI18n.changeLanguage("zh-CN");
    });
    expect(
      localizeAgentEffectFailureReason(
        {
          code: "unsafe_skill_directory",
          message:
            "Move the regular file that occupies the skill directory: .agents/skills/review",
          parameters: { relativePath: ".agents/skills/review" },
        },
        appI18n.t,
      ),
    ).toBe("技能目录路径不安全或被不受支持的条目占用：.agents/skills/review");
    expect(
      localizeAgentEffectFailureReason(
        {
          code: "managed_item_drift",
          message:
            "A Managed Item changed outside Ora and cannot be overwritten safely.",
          parameters: { relativePath: ".agents/skills/review" },
        },
        appI18n.t,
      ),
    ).toBe(
      "由 Ora 管理的技能目录在外部被修改，无法安全覆盖：.agents/skills/review",
    );
    expect(
      localizeAgentEffectFailureReason(
        {
          code: "managed_item_drift",
          message:
            "A Managed Item changed outside Ora and cannot be overwritten safely.",
          parameters: {},
        },
        appI18n.t,
      ),
    ).toBe("由 Ora 管理的技能目录在外部被修改，无法安全覆盖。");
    // Legacy persisted conflict rows without parameters preserve the backend safe message.
    expect(
      localizeAgentEffectFailureReason(
        {
          code: "preserved_item_conflict",
          message:
            "A Preserved Item already occupies the required native identity: foo.",
          parameters: {},
        },
        appI18n.t,
      ),
    ).toBe(
      "A Preserved Item already occupies the required native identity: foo.",
    );
    expect(
      localizeAgentEffectFailureReason(
        {
          code: "future_condition_code",
          message: "Backend safe detail.",
          parameters: {},
        },
        appI18n.t,
      ),
    ).toBe("Backend safe detail.");
    expect(
      localizeAgentEffectFailureSummary(
        {
          kind: "failed",
          message: "Effect recovery is required.",
          reasons: [],
          targetId: "target-1",
          recoveryRequired: true,
        },
        appI18n.t,
      ),
    ).toBe("存在未完成的技能同步操作，需要手动恢复。");
    expect(
      localizeAgentEffectFailureSummary(
        {
          kind: "failed",
          message:
            "Move the unmanaged directory that occupies the skill directory: .opencode/skills/foo",
          reasons: [
            {
              code: "preserved_item_conflict",
              message:
                "Move the unmanaged directory that occupies the skill directory: .opencode/skills/foo",
              parameters: { relativePath: ".opencode/skills/foo" },
            },
          ],
          targetId: "target-1",
          recoveryRequired: false,
        },
        appI18n.t,
      ),
    ).toBe("存在未由 Ora 管理的旧技能目录，无法覆盖：.opencode/skills/foo");
  });

  it("renders only when a failed Target has an action", () => {
    const client = createTestClient({});
    const ready = renderBanner({ kind: "ready" }, client);
    expect(ready.container).toBeEmptyDOMElement();
    ready.unmount();
    const blocked = renderBanner({ kind: "blocked" }, client);
    expect(blocked.container).toBeEmptyDOMElement();
    blocked.unmount();
    const retryable = renderBanner(
      {
        kind: "failed",
        message: "A Managed Item changed outside Ora.",
        reasons: [
          {
            code: "managed_item_drift",
            message: "A Managed Item changed outside Ora.",
            parameters: {},
          },
        ],
        targetId: "target-1",
        recoveryRequired: false,
      },
      client,
    );
    expect(retryable.container).not.toBeEmptyDOMElement();
    retryable.unmount();
    const unaddressable = renderBanner(
      {
        kind: "failed",
        message: "An Effect operation requires explicit recovery.",
        reasons: [],
        targetId: null,
        recoveryRequired: true,
      },
      client,
    );
    expect(unaddressable.container).toBeEmptyDOMElement();
  });

  it("keeps the quarantine visible and explains a refused recovery", async () => {
    const recoverEffectTarget = vi.fn(async () => {
      throw new Error(
        "operation operation-1 on Resource resource-1 proves neither exact state",
      );
    });
    const client = createTestClient({ recoverEffectTarget });

    renderBanner(quarantined, client);
    await userEvent.click(screen.getByRole("button", { name: /恢复|Recover/ }));

    const banner = await screen.findByRole("alert");
    expect(banner.dataset.effectFailure).toBe("failed");
    expect(banner).toHaveTextContent(/恢复未完成|Recovery did not complete/);
  });

  it("keeps the real send gate failed when recovery is refused", async () => {
    const ready = await readyEffectHandlers().getEffectTargetStatus();
    const getEffectTargetStatus = vi.fn(async () => ({
      status: {
        ...ready.status,
        targetId: "target-1",
        phase: "recovery_required" as const,
      },
    }));
    const client = createTestClient({
      ...pluginHandlers(createPluginMemory()),
      getEffectTargetStatus,
      recoverEffectTarget: async () => {
        throw new Error("recovery refused");
      },
    });
    function Gate() {
      const readiness = useAgentEffectReadiness(
        "workspace-1",
        AGENT_REF.claude,
      );
      return (
        <>
          <button aria-label="send" disabled={readiness.kind !== "ready"}>
            Send
          </button>
          <AgentEffectRecoveryBanner readiness={readiness} />
        </>
      );
    }
    render(<Gate />, {
      wrapper: createHookWrapper(
        client,
        createTestQueryClient(),
        createChatStore(client.session),
      ),
    });
    await screen.findByRole("alert");
    await userEvent.click(screen.getByRole("button", { name: /恢复|Recover/ }));
    await waitFor(() =>
      expect(screen.getByRole("alert").dataset.effectFailure).toBe("failed"),
    );
    expect(screen.getByRole("button", { name: "send" })).toBeDisabled();
    expect(getEffectTargetStatus).toHaveBeenCalledOnce();
  });

  it("exposes the pending state while recovery is in flight", async () => {
    // A holder keeps the resolver reachable after the closure assigns it; a bare
    // let-binding narrows to its null initializer across the callback boundary.
    const pending: {
      resolve: ((value: RecoverEffectTargetResponse) => void) | null;
    } = { resolve: null };
    const recoverEffectTarget = vi.fn(
      () =>
        new Promise<RecoverEffectTargetResponse>((resolve) => {
          pending.resolve = resolve;
        }),
    );
    const client = createTestClient({ recoverEffectTarget });

    renderBanner(quarantined, client);
    const banner = screen.getByRole("alert");
    expect(banner.dataset.effectFailure).toBe("required");
    const click = userEvent.click(
      screen.getByRole("button", { name: /恢复|Recover/ }),
    );
    await waitFor(() => expect(banner.dataset.effectFailure).toBe("pending"));
    pending.resolve?.({
      recovered: true,
      finalizedOperations: 1,
      closedWithoutEffectOperations: 0,
    });
    await click;
    await waitFor(() => expect(banner.dataset.effectFailure).toBe("required"));
  });
});
