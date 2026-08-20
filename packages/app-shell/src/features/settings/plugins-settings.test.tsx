import type { ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { PluginsSettings } from "./plugins-settings";

/** Renders plugin settings with isolated query and contracts-client state. */
function renderSettings(children: ReactNode) {
  const client = createMockClient(createMockClientState());
  client.agentRuntime.getStatus = async () => ({
    statuses: [
      { agentRef: "ora-space.opencode", status: "failing" },
      { agentRef: "ora-space.nga", status: "ready" },
      { agentRef: "ora-space.codeagentcli", status: "ready" },
    ],
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>{children}</AppI18nProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
}

/** A circuit-open agent is presented as failed instead of as an actionable install button. */
it("shows the terminal runtime failure reported by the backend", async () => {
  renderSettings(<PluginsSettings />);

  expect(
    await screen.findByRole("button", {
      name: /运行失败|Runtime failed/,
    }),
  ).toBeDisabled();
});
