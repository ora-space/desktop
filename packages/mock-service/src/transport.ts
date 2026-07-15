import type { ContractTransport, ContractTransportRequest } from "@ora/contracts";
import { createFetchTransport } from "@ora/contracts/fetch";

let workerStartPromise: Promise<void> | undefined;

/** Starts MSW once and waits until requests can safely be intercepted. */
function ensureWorkerStarted(): Promise<void> {
  if (workerStartPromise !== undefined) {
    return workerStartPromise;
  }

  workerStartPromise = (async () => {
    if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) {
      throw new Error("mock service requires browser Service Worker support");
    }

    const { worker } = await import("./browser.js");
    await worker.start({
      onUnhandledRequest: "bypass",
      serviceWorker: { url: "/mockServiceWorker.js" },
    });
  })();

  return workerStartPromise;
}

/** Restores contract runtime types that JSON cannot represent directly. */
export function reviveMockResponse(operationName: string, response: unknown): unknown {
  if (
    operationName !== "openProjectWorkContext" &&
    operationName !== "renewProjectWorkContext"
  ) {
    return response;
  }
  if (typeof response !== "object" || response === null || !("context" in response)) {
    return response;
  }

  const context = response.context;
  if (
    typeof context !== "object" ||
    context === null ||
    !("leaseExpiresAt" in context) ||
    (typeof context.leaseExpiresAt !== "number" && typeof context.leaseExpiresAt !== "string")
  ) {
    return response;
  }

  return {
    ...response,
    context: {
      ...context,
      leaseExpiresAt: BigInt(context.leaseExpiresAt),
    },
  };
}

/** Creates a contracts transport whose HTTP traffic is intercepted by the mock Service Worker. */
export function createMockTransport(): ContractTransport {
  const fetchTransport = createFetchTransport();

  return {
    async send<TResponse>(request: ContractTransportRequest): Promise<TResponse> {
      await ensureWorkerStarted();
      const response = await fetchTransport.send<unknown>(request);

      return reviveMockResponse(request.operationName, response) as TResponse;
    },
  };
}
