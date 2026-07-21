import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { createMockAcpClient, createMockTransport, mockChatSuggestions, mockCurrentUser } from "@ora/mock-service";
import { createWebPlatformAdapter } from "@ora/platform/web";

export const client = createContractsClient(createMockTransport());
export const chatStore = createChatStore(createMockAcpClient());
export const platform = createWebPlatformAdapter(client);
export const chatSuggestions = mockChatSuggestions;
export const currentUser = mockCurrentUser;
