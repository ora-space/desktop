import { createElement, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, type RenderHookResult } from "@testing-library/react";
import type { ContractsClient } from "@ora/contracts";
import { createChatStore, type ChatStore } from "@ora/chat";
import { ContractsClientContext } from "../contracts-client-context";
import { ChatStoreContext } from "../chat-store-context";
import type { WorkflowRuntime } from "@ora/workflow-runtime";
import { createMemoryWorkflowRuntime } from "@ora/workflow-runtime/memory";
import { WorkflowRuntimeProvider } from "../features/workflow-run/workflow-runtime-context";
import { AppI18nProvider } from "../i18n/i18n";

/** Builds a QueryClient with retries disabled so tests fail fast on transport errors. */
export function createTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

/** Wraps children with the application providers used by hook implementations. */
export function createHookWrapper(
  client: ContractsClient,
  queryClient: QueryClient,
  chatStore: ChatStore,
  runtime: WorkflowRuntime = createMemoryWorkflowRuntime(),
) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(
      AppI18nProvider,
      null,
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          WorkflowRuntimeProvider,
          {
            runtime,
            children: createElement(
              ContractsClientContext.Provider,
              { value: client },
              createElement(ChatStoreContext.Provider, { value: chatStore }, children),
            ),
          },
        ),
      ),
    );
  };
}

/** Renders a hook with both providers set up and returns the result + QueryClient. */
export function renderHookWithClient<TResult>(
  hook: () => TResult,
  client: ContractsClient,
  queryClient: QueryClient = createTestQueryClient(),
  chatStore: ChatStore = createChatStore(client.session),
  runtime: WorkflowRuntime = createMemoryWorkflowRuntime(),
): RenderHookResult<TResult, TResult> & { queryClient: QueryClient } {
  const result = renderHook(hook, {
    wrapper: createHookWrapper(client, queryClient, chatStore, runtime),
  });
  return { ...result, queryClient };
}
