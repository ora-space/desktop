import { createContractsClient } from "@ora/contracts";
import { createFetchTransport } from "@ora/contracts/fetch";
import { createWebPlatformAdapter } from "@ora/platform/web";

export const client = createContractsClient(createFetchTransport());
export const platform = createWebPlatformAdapter(client);
