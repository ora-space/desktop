import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement } from "react";
import type { ContractsClient } from "@ora/contracts";
import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { createTestClient } from "../../test/contracts-transport";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { createChatStore } from "@ora/chat";
import "../../i18n/i18n-instance";
import type { RecoverEffectTargetResponse } from "@ora/contracts";
import type { AgentEffectReadiness } from "../../state/hooks/use-agent-effect-readiness";
import { AgentEffectRecoveryBanner } from "./agent-effect-recovery-banner";

/** The failed readiness a quarantined Effect Target produces. */
const quarantined: AgentEffectReadiness = {
  kind: "failed",
  message: "An Effect operation requires explicit recovery.",
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
    ...render(createElement(AgentEffectRecoveryBanner, { readiness }), { wrapper }),
    queries,
  };
}

describe("AgentEffectRecoveryBanner", () => {
  it("renders nothing unless the failure is the manual-recovery quarantine", () => {
    const client = createTestClient({});
    const { container } = renderBanner(
      { kind: "ready" },
      client,
    );
    expect(container).toBeEmptyDOMElement();
    renderBanner(
      { kind: "blocked" },
      client,
    );
    renderBanner(
      {
        kind: "failed",
        message: "A Managed Item changed outside Ora.",
        targetId: "target-1",
        recoveryRequired: false,
      },
      client,
    );
    renderBanner(
      {
        kind: "failed",
        message: "An Effect operation requires explicit recovery.",
        targetId: null,
        recoveryRequired: true,
      },
      client,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("recovers the quarantined Target and refreshes the send-gate status", async () => {
    const recoverEffectTarget = vi.fn(
      async (request: {
        targetId: string;
      }): Promise<RecoverEffectTargetResponse> => {
        expect(request).toEqual({ targetId: "target-1" });
        return {
          recovered: true,
          finalizedOperations: 1,
          closedWithoutEffectOperations: 0,
        };
      },
    );
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const client = createTestClient({ recoverEffectTarget });

    renderBanner(quarantined, client, queryClient);
    await userEvent.click(screen.getByRole("button", { name: /恢复|Recover/ }));

    await waitFor(() => expect(recoverEffectTarget).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: ["agent-effect-status"],
      }),
    );
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
    expect(banner.dataset.effectRecovery).toBe("failed");
    expect(banner).toHaveTextContent(/恢复未完成|Recovery did not complete/);
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
    expect(banner.dataset.effectRecovery).toBe("required");
    const click = userEvent.click(
      screen.getByRole("button", { name: /恢复|Recover/ }),
    );
    await waitFor(() => expect(banner.dataset.effectRecovery).toBe("pending"));
    pending.resolve?.({
      recovered: true,
      finalizedOperations: 1,
      closedWithoutEffectOperations: 0,
    });
    await click;
    await waitFor(() => expect(banner.dataset.effectRecovery).toBe("required"));
  });
});
