import type {
  AgentBusinessErrorData,
  AgentBusinessFailureKind,
  AgentEvent,
  CancelConversationRequest,
  CancelConversationResponse,
  DiscoverInstallationsRequest,
  DiscoverInstallationsResponse,
  GetConfigurationSummaryRequest,
  GetConfigurationSummaryResponse,
  JsonValue,
  ListConversationsRequest,
  ListConversationsResponse,
  ListMcpServersRequest,
  ListMcpServersResponse,
  ListSkillsRequest,
  ListSkillsResponse,
  SendMessageRequest,
  StartConversationRequest,
  AgentTurnResult,
} from "../types/index.js";

export type { AgentBusinessErrorData, AgentBusinessFailureKind, AgentEvent, JsonValue };

export interface AgentPluginDefinition {
  readonly kind: "agent";
  readonly pluginApi: 1;
  activate(context: ExtensionContext): AgentActivation | Promise<AgentActivation>;
  deactivate?(): void | Promise<void>;
}

export interface AgentActivation {
  readonly providers: readonly AgentProvider[];
}

export interface ExtensionContext {
  readonly plugin: Readonly<{ id: string; version: string }>;
  readonly sessionId: string;
  readonly extensionPath: string;
  readonly storagePath: string;
  readonly logger: PluginLogger;
  readonly shutdownSignal: AbortSignal;
  readonly subscriptions: SubscriptionStore;
  readonly errors: Readonly<{
    business(input: AgentBusinessErrorInput): AgentBusinessError;
  }>;
}

export type AuthorBusinessFailureKind = Exclude<
  AgentBusinessFailureKind,
  "providerFailure"
>;

export interface AgentBusinessErrorInput {
  readonly kind: AuthorBusinessFailureKind;
  readonly message: string;
  readonly retryable?: boolean;
  readonly details?: Readonly<Record<string, JsonValue>>;
}

export interface AgentBusinessError extends Error {
  readonly name: "AgentBusinessError";
  readonly kind: AuthorBusinessFailureKind;
  readonly retryable: boolean;
  readonly details?: Readonly<Record<string, JsonValue>>;
}

export interface PluginLogger {
  debug(message: string): void;
  info(message: string): void;
  warn(message: string): void;
  error(message: string): void;
}

export interface Disposable {
  dispose(): void | Promise<void>;
}

export interface SubscriptionStore {
  add<T extends Disposable>(disposable: T): T;
}

export interface AgentCallContext {
  readonly requestId: string;
  readonly signal: AbortSignal;
}

export interface AgentProvider {
  readonly id: string;
  readonly contractVersion: 1;
  discoverInstallations(
    call: AgentCallContext,
    request: DiscoverInstallationsRequest,
  ): Promise<DiscoverInstallationsResponse>;
  getConfigurationSummary(
    call: AgentCallContext,
    request: GetConfigurationSummaryRequest,
  ): Promise<GetConfigurationSummaryResponse>;
  listSkills(call: AgentCallContext, request: ListSkillsRequest): Promise<ListSkillsResponse>;
  listMcpServers(
    call: AgentCallContext,
    request: ListMcpServersRequest,
  ): Promise<ListMcpServersResponse>;
  listConversations(
    call: AgentCallContext,
    request: ListConversationsRequest,
  ): Promise<ListConversationsResponse>;
  startConversation(
    call: AgentCallContext,
    request: StartConversationRequest,
  ): AsyncGenerator<AgentEvent, AgentTurnResult, void>;
  sendMessage(
    call: AgentCallContext,
    request: SendMessageRequest,
  ): AsyncGenerator<AgentEvent, AgentTurnResult, void>;
  cancelConversation(
    call: AgentCallContext,
    request: CancelConversationRequest,
  ): Promise<CancelConversationResponse>;
}

/** Returns the same structural definition without I/O, registration, or global state. */
export function defineAgentPlugin<T extends AgentPluginDefinition>(definition: T): T {
  return definition;
}
