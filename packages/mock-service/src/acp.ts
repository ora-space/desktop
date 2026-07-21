import type {
  AcpClient,
  AcpSessionNotificationListener,
} from "@ora/chat";
import type { acp } from "@ora/contracts";
import {
  defaultMockAcpScenarioResolver,
  type MockAcpScenarioResolver,
} from "./acp-scenario.js";
import { MockVirtualFileSystem, mockFileFixtures } from "./virtual-files.js";

const DEFAULT_CHUNK_SIZE = 8;
const DEFAULT_CHUNK_DELAY_MS = 80;
const DEFAULT_STEP_DELAY_MS = 180;
const SEEDED_AGENT_SESSION_ID = "agent-session-runtime";

/** Waits between mock events so production-like streaming remains testable. */
export interface MockAcpScheduler {
  wait(delayMs: number): Promise<void>;
}

/** Selects a failure the mock agent injects into `prompt`. */
export type MockAcpFault =
  | { kind: "failBeforeStream"; message: string }
  | { kind: "failMidStream"; afterChunks: number; message: string }
  | { kind: "hang" };

/** Configures deterministic mock ACP timing, identity generation, scenarios, and failures. */
export interface MockAcpClientOptions {
  scheduler?: MockAcpScheduler;
  chunkSize?: number;
  chunkDelayMs?: number;
  stepDelayMs?: number;
  createId?: () => string;
  initialSessionIds?: Iterable<string>;
  stopReason?: acp.StopReason;
  fault?: MockAcpFault;
  scenarioResolver?: MockAcpScenarioResolver;
}

interface ActivePrompt {
  cancelled: boolean;
  activeToolId: string | null;
  cancelSignal: Promise<void>;
  resolveCancel(): void;
}

interface PromptContext {
  sessionId: string;
  promptText: string;
  control: ActivePrompt;
  listeners: Set<AcpSessionNotificationListener>;
  scheduler: MockAcpScheduler;
  chunkSize: number;
  chunkDelayMs: number;
  stepDelayMs: number;
  createId: () => string;
  fileSystem: MockVirtualFileSystem;
  fault?: MockAcpFault;
  emittedChunks: number;
}

class CancelledPrompt extends Error {
  constructor() {
    super("ACP prompt cancelled");
  }
}

const timeoutScheduler: MockAcpScheduler = {
  wait: (delayMs) =>
    new Promise((resolve) => {
      setTimeout(resolve, delayMs);
    }),
};

/** Creates an in-memory ACP agent with deterministic chat and tool execution scenarios. */
export function createMockAcpClient(
  options: MockAcpClientOptions = {},
): AcpClient {
  const scheduler = options.scheduler ?? timeoutScheduler;
  const chunkSize = options.chunkSize ?? DEFAULT_CHUNK_SIZE;
  const chunkDelayMs = options.chunkDelayMs ?? DEFAULT_CHUNK_DELAY_MS;
  const stepDelayMs = options.stepDelayMs ?? DEFAULT_STEP_DELAY_MS;
  const createId = options.createId ?? (() => crypto.randomUUID());
  const stopReason = options.stopReason ?? "end_turn";
  const scenarioResolver = options.scenarioResolver ?? defaultMockAcpScenarioResolver;
  const sessionIds = new Set(options.initialSessionIds ?? [SEEDED_AGENT_SESSION_ID]);
  const activePrompts = new Map<string, ActivePrompt>();
  const listeners = new Set<AcpSessionNotificationListener>();
  const fileSystem = new MockVirtualFileSystem();

  for (const sessionId of sessionIds) fileSystem.ensureSession(sessionId);

  if (!Number.isInteger(chunkSize) || chunkSize <= 0) {
    throw new Error("mock ACP chunkSize must be a positive integer");
  }

  return {
    async newSession(request) {
      const sessionId = `agent-session-${createId()}`;
      sessionIds.add(sessionId);
      fileSystem.createSession(sessionId, request.cwd);
      return { sessionId };
    },

    async prompt(request) {
      if (!sessionIds.has(request.sessionId)) {
        throw new Error(`ACP session not found: ${request.sessionId}`);
      }
      if (activePrompts.has(request.sessionId)) {
        throw new Error(`ACP session is already processing a prompt: ${request.sessionId}`);
      }
      if (options.fault?.kind === "failBeforeStream") {
        throw new Error(options.fault.message);
      }

      const promptText = request.prompt.filter(isTextContent).map((block) => block.text).join("\n");
      const control = createActivePrompt();
      const context: PromptContext = {
        sessionId: request.sessionId,
        promptText,
        control,
        listeners,
        scheduler,
        chunkSize,
        chunkDelayMs,
        stepDelayMs,
        createId,
        fileSystem,
        ...(options.fault === undefined ? {} : { fault: options.fault }),
        emittedChunks: 0,
      };
      activePrompts.set(request.sessionId, control);

      try {
        const scenario = scenarioResolver(promptText);
        switch (scenario) {
          case "chat":
            await streamAgentText(context, `Mock response: ${promptText}`);
            break;
          case "tool_success":
            await runSuccessfulToolScenario(context);
            break;
          case "tool_failure":
            await runFailedToolScenario(context);
            break;
        }

        if (options.fault?.kind === "hang") {
          await control.cancelSignal;
          throw new CancelledPrompt();
        }

        return { stopReason };
      } catch (error) {
        if (!(error instanceof CancelledPrompt)) throw error;
        emitCancelledTool(context);
        return { stopReason: "cancelled" };
      } finally {
        activePrompts.delete(request.sessionId);
      }
    },

    async cancel(notification) {
      const control = activePrompts.get(notification.sessionId);
      if (control === undefined || control.cancelled) return;
      control.cancelled = true;
      control.resolveCancel();
    },

    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

/** Streams a normal assistant response in stable chunks. */
async function streamAgentText(context: PromptContext, text: string): Promise<void> {
  await streamContent(context, "agent_message_chunk", `agent-message-${context.createId()}`, text);
}

/** Emits a complete multi-read, multi-edit, and verification workflow against virtual fixtures. */
async function runSuccessfulToolScenario(context: PromptContext): Promise<void> {
  await streamContent(
    context,
    "agent_thought_chunk",
    `agent-thought-${context.createId()}`,
    "I will inspect the relevant files, update the implementation, and verify the result.",
  );

  const plan = createPlanEntries();
  emitPlan(context, plan);

  await executeReadTool(context, mockFileFixtures.appPath, "Read the implementation");
  advancePlan(context, plan, 0);

  await executeReadTool(context, mockFileFixtures.testPath, "Read the existing tests");
  advancePlan(context, plan, 1);

  await executeEditTool(context, {
    relativePath: mockFileFixtures.appPath,
    title: "Update the greeting implementation",
    operation: "normalize the supplied name",
    newText: mockFileFixtures.updatedAppSource,
  });
  advancePlan(context, plan, 2);

  await executeEditTool(context, {
    relativePath: mockFileFixtures.testPath,
    title: "Add whitespace coverage",
    operation: "cover normalized names",
    newText: mockFileFixtures.updatedTestSource,
  });
  advancePlan(context, plan, 3);

  await executeMockCommand(context, {
    title: "Run TypeScript checks",
    command: "pnpm exec tsc --noEmit",
    output: "TypeScript checks passed.\n",
  });
  await executeMockCommand(context, {
    title: "Run greeting tests",
    command: "pnpm test -- greeting",
    output: "2 tests passed.\n",
  });
  advancePlan(context, plan, 4);

  await streamAgentText(context, "Updated the greeting implementation and tests, then completed type checking and test verification.");
}

/** Emits a failed read while keeping the ACP turn itself healthy. */
async function runFailedToolScenario(context: PromptContext): Promise<void> {
  await streamContent(
    context,
    "agent_thought_chunk",
    `agent-thought-${context.createId()}`,
    "I will inspect the requested file before making any changes.",
  );
  const plan: acp.PlanEntry[] = [
    { content: "Inspect the requested file", priority: "high", status: "in_progress" },
    { content: "Apply the requested change", priority: "medium", status: "pending" },
  ];
  emitPlan(context, plan);

  const path = "src/missing.ts";
  const absolutePath = context.fileSystem.absolutePath(context.sessionId, path);
  const toolCallId = `tool-${context.createId()}`;
  context.control.activeToolId = toolCallId;
  emitUpdate(context, {
    sessionUpdate: "tool_call",
    toolCallId,
    title: "Read the requested file",
    kind: "read",
    status: "pending",
    locations: [{ path: absolutePath }],
    rawInput: { path: absolutePath },
  });
  await waitForStage(context);
  emitUpdate(context, { sessionUpdate: "tool_call_update", toolCallId, status: "in_progress" });
  await waitForStage(context);
  emitUpdate(context, {
    sessionUpdate: "tool_call_update",
    toolCallId,
    status: "failed",
    rawOutput: { error: `File not found: ${absolutePath}` },
  });
  context.control.activeToolId = null;
  await streamAgentText(context, `I could not continue because ${absolutePath} does not exist.`);
}

/** Runs one read tool through pending, running, and completed states. */
async function executeReadTool(context: PromptContext, relativePath: string, title: string): Promise<void> {
  const absolutePath = context.fileSystem.absolutePath(context.sessionId, relativePath);
  const contents = context.fileSystem.read(context.sessionId, relativePath);
  if (contents === undefined) throw new Error(`virtual fixture not found: ${relativePath}`);
  const toolCallId = `tool-${context.createId()}`;
  context.control.activeToolId = toolCallId;
  emitUpdate(context, {
    sessionUpdate: "tool_call",
    toolCallId,
    title,
    kind: "read",
    status: "pending",
    locations: [{ path: absolutePath, line: 1 }],
    rawInput: { path: absolutePath },
  });
  await waitForStage(context);
  emitUpdate(context, { sessionUpdate: "tool_call_update", toolCallId, status: "in_progress" });
  await waitForStage(context);
  emitUpdate(context, {
    sessionUpdate: "tool_call_update",
    toolCallId,
    status: "completed",
    content: [{ type: "content", content: { type: "text", text: contents } }],
    rawOutput: { bytes: contents.length },
  });
  context.control.activeToolId = null;
}

interface MockEdit {
  relativePath: string;
  title: string;
  operation: string;
  newText: string;
}

/** Runs one parameterized edit so fixture-specific details remain inside the mock workflow. */
async function executeEditTool(context: PromptContext, edit: MockEdit): Promise<void> {
  const { relativePath } = edit;
  const absolutePath = context.fileSystem.absolutePath(context.sessionId, relativePath);
  const oldText = context.fileSystem.read(context.sessionId, relativePath);
  if (oldText === undefined) throw new Error(`virtual fixture not found: ${relativePath}`);
  const toolCallId = `tool-${context.createId()}`;
  context.control.activeToolId = toolCallId;
  emitUpdate(context, {
    sessionUpdate: "tool_call",
    toolCallId,
    title: edit.title,
    kind: "edit",
    status: "pending",
    locations: [{ path: absolutePath, line: 1 }],
    rawInput: { path: absolutePath, operation: edit.operation },
  });
  await waitForStage(context);
  emitUpdate(context, { sessionUpdate: "tool_call_update", toolCallId, status: "in_progress" });
  await waitForStage(context);
  context.fileSystem.write(context.sessionId, relativePath, edit.newText);
  emitUpdate(context, {
    sessionUpdate: "tool_call_update",
    toolCallId,
    status: "completed",
    content: [{
      type: "diff",
      path: absolutePath,
      oldText,
      newText: edit.newText,
    }],
    rawOutput: { changed: true },
  });
  context.control.activeToolId = null;
}

interface MockCommand {
  title: string;
  command: string;
  output: string;
}

/** Simulates a successful command lifecycle without invoking the host shell. */
async function executeMockCommand(context: PromptContext, command: MockCommand): Promise<void> {
  const toolCallId = `tool-${context.createId()}`;
  context.control.activeToolId = toolCallId;
  emitUpdate(context, {
    sessionUpdate: "tool_call",
    toolCallId,
    title: command.title,
    kind: "execute",
    status: "pending",
    rawInput: { command: command.command },
  });
  await waitForStage(context);
  emitUpdate(context, { sessionUpdate: "tool_call_update", toolCallId, status: "in_progress" });
  await waitForStage(context);
  emitUpdate(context, {
    sessionUpdate: "tool_call_update",
    toolCallId,
    status: "completed",
    content: [{ type: "content", content: { type: "text", text: command.output } }],
    rawOutput: { exitCode: 0 },
  });
  context.control.activeToolId = null;
}

/** Creates the complete plan snapshot used by the successful scenario. */
function createPlanEntries(): acp.PlanEntry[] {
  return [
    { content: "Inspect the implementation", priority: "high", status: "in_progress" },
    { content: "Inspect the existing tests", priority: "medium", status: "pending" },
    { content: "Update the implementation", priority: "high", status: "pending" },
    { content: "Add regression coverage", priority: "high", status: "pending" },
    { content: "Run type checking and tests", priority: "high", status: "pending" },
  ];
}

/** Completes one plan entry and starts the next before publishing a replacement snapshot. */
function advancePlan(context: PromptContext, plan: acp.PlanEntry[], completedIndex: number): void {
  plan[completedIndex] = { ...plan[completedIndex]!, status: "completed" };
  const nextEntry = plan[completedIndex + 1];
  if (nextEntry !== undefined) plan[completedIndex + 1] = { ...nextEntry, status: "in_progress" };
  emitPlan(context, plan);
}

/** Streams one ACP content type while respecting faults and cancellation. */
async function streamContent(
  context: PromptContext,
  sessionUpdate: "agent_message_chunk" | "agent_thought_chunk",
  messageId: string,
  text: string,
): Promise<void> {
  for (const chunk of splitText(text, context.chunkSize)) {
    await waitForDelay(context, context.chunkDelayMs);
    emitUpdate(context, {
      sessionUpdate,
      messageId,
      content: { type: "text", text: chunk },
    });
    context.emittedChunks += 1;
    if (
      context.fault?.kind === "failMidStream"
      && context.emittedChunks >= context.fault.afterChunks
    ) {
      throw new Error(context.fault.message);
    }
  }
}

/** Emits a complete plan snapshot because ACP plan notifications replace prior state. */
function emitPlan(context: PromptContext, entries: acp.PlanEntry[]): void {
  emitUpdate(context, { sessionUpdate: "plan", entries: entries.map((entry) => ({ ...entry })) });
}

/** Waits for a visible tool stage while allowing cancellation to interrupt it. */
async function waitForStage(context: PromptContext): Promise<void> {
  await waitForDelay(context, context.stepDelayMs);
}

/** Races one scheduler wait against the active prompt's cancellation signal. */
async function waitForDelay(context: PromptContext, delayMs: number): Promise<void> {
  await Promise.race([context.scheduler.wait(delayMs), context.control.cancelSignal]);
  if (context.control.cancelled) throw new CancelledPrompt();
}

/** Marks the currently visible tool as cancelled before the turn ends. */
function emitCancelledTool(context: PromptContext): void {
  const toolCallId = context.control.activeToolId;
  if (toolCallId === null) return;
  emitUpdate(context, {
    sessionUpdate: "tool_call_update",
    toolCallId,
    status: "failed",
    rawOutput: { error: "Cancelled by user" },
  });
  context.control.activeToolId = null;
}

/** Builds the deferred signal used to interrupt any mock stage. */
function createActivePrompt(): ActivePrompt {
  let resolveCancel = (): void => undefined;
  const cancelSignal = new Promise<void>((resolve) => {
    resolveCancel = resolve;
  });
  return { cancelled: false, activeToolId: null, cancelSignal, resolveCancel };
}

/** Narrows prompt blocks to the baseline text content supported by this mock. */
function isTextContent(
  block: acp.ContentBlock,
): block is Extract<acp.ContentBlock, { type: "text" }> {
  return block.type === "text";
}

/** Splits text into stable chunks without losing whitespace or punctuation. */
function splitText(text: string, chunkSize: number): string[] {
  const chunks: string[] = [];
  for (let offset = 0; offset < text.length; offset += chunkSize) {
    chunks.push(text.slice(offset, offset + chunkSize));
  }
  return chunks;
}

/** Delivers one typed update to a snapshot so listeners may unsubscribe safely. */
function emitUpdate(context: PromptContext, update: acp.SessionUpdate): void {
  const notification: acp.SessionNotification = { sessionId: context.sessionId, update };
  [...context.listeners].forEach((listener) => listener(notification));
}
