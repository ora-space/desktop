import { createElement, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, type RenderHookResult } from "@testing-library/react";
import type { ContractsClient } from "@ora/contracts";
import { createChatStore, type AcpClient, type ChatStore } from "@ora/chat";
import { ContractsClientContext } from "../contracts-client-context";
import { ChatStoreContext } from "../chat-store-context";

const testAcpClient: AcpClient = {
  newSession: async () => ({ sessionId: "agent-session-test" }),
  prompt: async () => ({ stopReason: "end_turn" }),
  cancel: async () => undefined,
  subscribe: () => () => undefined,
};

/** Builds a QueryClient with retries disabled so tests fail fast on transport errors. */
export function createTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

/** Wraps children with QueryClient + ContractsClient providers for hook tests. */
export function createHookWrapper(
  client: ContractsClient,
  queryClient: QueryClient,
  chatStore: ChatStore,
) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        createElement(ChatStoreContext.Provider, { value: chatStore }, children),
      ),
    );
  };
}

/** Renders a hook with both providers set up and returns the result + QueryClient. */
export function renderHookWithClient<TResult>(
  hook: () => TResult,
  client: ContractsClient,
  queryClient: QueryClient = createTestQueryClient(),
  chatStore: ChatStore = createChatStore(testAcpClient),
): RenderHookResult<TResult, TResult> & { queryClient: QueryClient } {
  const result = renderHook(hook, { wrapper: createHookWrapper(client, queryClient, chatStore) });
  return { ...result, queryClient };
}
