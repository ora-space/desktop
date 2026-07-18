import { createContractsClient } from "@ora/contracts";
import { createMockTransport } from "@ora/mock-service";
import { createWebPlatformAdapter } from "@ora/platform/web";

export const client = createContractsClient(createMockTransport());
export const platform = createWebPlatformAdapter(client);
