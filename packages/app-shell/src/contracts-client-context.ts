import { createContext, useContext } from "react";
import type { ContractsClient } from "@ora/contracts";

export const ContractsClientContext = createContext<ContractsClient | null>(
  null,
);

/** Returns the backend client injected at the application-shell boundary. */
export function useContractsClient(): ContractsClient {
  const client = useContext(ContractsClientContext);
  if (client === null) {
    throw new Error("useContractsClient must be used within AppShell");
  }

  return client;
}

/** Returns the backend client when present, or null when rendering in lightweight test harnesses. */
export function useOptionalContractsClient(): ContractsClient | null {
  return useContext(ContractsClientContext);
}
