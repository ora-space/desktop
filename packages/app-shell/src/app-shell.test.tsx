import { render, waitFor } from "@testing-library/react";
import { createChatStore } from "@ora/chat";
import type { AppEvent } from "@ora/contracts";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppEventGate } from "./state/app-event-gate";
import { createMockClient, createMockClientState } from "./test/mock-client";
import { createHookWrapper, createTestQueryClient } from "./test/hook-harness";

afterEach(() => {
  vi.useRealTimers();
});

/** Waits until the stream is cancelled without adding a second reconnect source to the test. */
function waitForAbort(signal: AbortSignal | undefined): Promise<void> {
  return new Promise((resolve) => {
    if (signal === undefined || signal.aborted) {
      resolve();
      return;
    }
    signal.addEventListener("abort", () => resolve(), { once: true });
  });
}

describe("AppEventGate reconnect behavior", () => {
  it("refetches and backs off after a stream ends, then resets after Ready", async () => {
    const client = createMockClient(createMockClientState());
    let attempts = 0;
    client.appEvents.watch = async function* (
      _request,
      options,
    ): AsyncGenerator<AppEvent> {
      attempts += 1;
      yield { type: "ready" };
      if (attempts === 1) return;
      await waitForAbort(options?.signal);
    };
    const queryClient = createTestQueryClient();
    const refetch = vi.spyOn(queryClient, "refetchQueries");
    const Wrapper = createHookWrapper(
      client,
      queryClient,
      createChatStore(client.session),
    );
    const { unmount } = render(
      <Wrapper>
        <AppEventGate client={client}>
          <div data-testid="business-content">business workload</div>
        </AppEventGate>
      </Wrapper>,
    );

    await waitFor(() => expect(refetch).toHaveBeenCalledTimes(2));
    expect(attempts).toBe(1);
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(attempts).toBe(1);
    await waitFor(() => expect(attempts).toBe(2), { timeout: 2_500 });
    expect(refetch).toHaveBeenCalledTimes(3);

    unmount();
  });
});
