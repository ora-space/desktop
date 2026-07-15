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

/** Creates a contracts transport whose HTTP traffic is intercepted by the mock Service Worker. */
export function createMockTransport(): ContractTransport {
  const fetchTransport = createFetchTransport();

  return {
    async send<TResponse>(request: ContractTransportRequest): Promise<TResponse> {
      await ensureWorkerStarted();
      return fetchTransport.send<TResponse>(request);
    },
  };
}
