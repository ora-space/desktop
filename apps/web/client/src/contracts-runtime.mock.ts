import { createChatStore } from "@ora/chat";
import {
  createMemoryContractsClient,
  createMemoryContractsState,
} from "@ora/contracts/memory";
import { createWebPlatformAdapter } from "@ora/platform/web";

export const contractTransport = "mock" as const;
export const client = createMemoryContractsClient(
  createMemoryContractsState({
    projects: [{
      id: "p1",
      name: "Workflow Prototype",
      rootPath: "/workspace/workflow-prototype",
    }],
  }),
);
export const chatStore = createChatStore(client.session);
export const platform = createWebPlatformAdapter(client);
