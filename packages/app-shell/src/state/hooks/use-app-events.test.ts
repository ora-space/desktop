import { waitFor } from "@testing-library/react";
import type { AppEvent } from "@ora/contracts";
import { describe, expect, it, vi } from "vitest";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { createTestQueryClient, renderHookWithClient } from "../../test/hook-harness";
import { queryKeys } from "./query-keys";
import { useAppEvents } from "./use-app-events";

describe("useAppEvents", () => {
  it("refetches after Ready and invalidates sessions for title events", async () => {
    const client = createMockClient(createMockClientState());
    client.appEvents.watch = async function* (_request, options): AsyncGenerator<AppEvent> {
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

    const { result, unmount } = renderHookWithClient(() => useAppEvents(client), client, queryClient);

    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(refetch).toHaveBeenCalledWith({ queryKey: queryKeys.sessions });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.sessions });

    unmount();
  });
});
