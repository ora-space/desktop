import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import { RemoteContractError, type AppEvent } from "@ora/contracts";
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

/** Creates the stable remote error returned while another document owns the backend lease. */
function multipleClientsError(): RemoteContractError {
  return new RemoteContractError(
    {
      requestId: "app-event-test",
      code: "multiple_clients_unsupported",
      params: {},
    },
    409,
    null,
  );
}

describe("AppEventGate", () => {
  it("blocks business content and only retries after the user asks", async () => {
    const client = createMockClient(createMockClientState());
    let attempts = 0;
    client.appEvents.watch = async function* (_request, options): AsyncGenerator<AppEvent> {
      attempts += 1;
      if (attempts === 1) {
        throw multipleClientsError();
      }
      yield { type: "ready" };
      await waitForAbort(options?.signal);
    };
    const Wrapper = createHookWrapper(client, createTestQueryClient(), createChatStore(client.session));
    // The gate itself only needs the query and i18n providers; use the normal wrapper so its
    // children cannot accidentally initialize application hooks outside the production tree.
    render(
      <Wrapper>
        <AppEventGate client={client}>
          <div data-testid="business-content">business workload</div>
        </AppEventGate>
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "应用已在其他页面打开" })).toBeInTheDocument();
    });
    expect(screen.queryByTestId("business-content")).not.toBeInTheDocument();
    expect(attempts).toBe(1);

    await userEvent.click(screen.getByRole("button", { name: "重新检查" }));
    await waitFor(() => expect(screen.getByTestId("business-content")).toBeInTheDocument());
    expect(attempts).toBe(2);
  });
});

describe("AppEventGate reconnect behavior", () => {
  it("refetches and backs off after a stream ends, then resets after Ready", async () => {
    const client = createMockClient(createMockClientState());
    let attempts = 0;
    client.appEvents.watch = async function* (_request, options): AsyncGenerator<AppEvent> {
      attempts += 1;
      yield { type: "ready" };
      if (attempts === 1) return;
      await waitForAbort(options?.signal);
    };
    const queryClient = createTestQueryClient();
    const refetch = vi.spyOn(queryClient, "refetchQueries");
    const Wrapper = createHookWrapper(client, queryClient, createChatStore(client.session));
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
