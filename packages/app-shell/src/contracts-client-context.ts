import { createContext, useContext } from "react";
import type { ContractsClient } from "@ora/contracts";

export const ContractsClientContext = createContext<ContractsClient | null>(null);

/** Returns the backend client injected at the application-shell boundary. */
export function useContractsClient(): ContractsClient {
  const client = useContext(ContractsClientContext);
  if (client === null) {
    throw new Error("useContractsClient must be used within AppShell");
  }

  return client;
}
