import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { createFetchTransport } from "@ora/contracts/fetch";
import { createWebPlatformAdapter } from "@ora/platform/web";

export const contractTransport = "fetch" as const;
export const client = createContractsClient(createFetchTransport());
export const chatStore = createChatStore(client.session);
export const platform = createWebPlatformAdapter(client);
