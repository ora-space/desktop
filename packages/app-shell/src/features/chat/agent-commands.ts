import type * as acp from "@agentclientprotocol/sdk";
import type { AgentCommand } from "@ora/contracts";

/** Limits on agent-written command text, applied to what is shown and never to what is delivered. */
const MAX_COMMANDS = 500;
const MAX_NAME_BYTES = 128;
const MAX_DESCRIPTION_BYTES = 4096;
const MAX_HINT_BYTES = 256;

/**
 * The commands of the agent the next message goes to, as discovered outside any session.
 *
 * They only fill the slash menu. Discovery runs without MCP servers, so a name missing here says
 * nothing about whether the agent supports it.
 */
export interface DiscoveredCommands {
  agentRef: string;
  /** `null` when the agent's commands are unknown, which is not the same as it having none. */
  commands: acp.AvailableCommand[] | null;
  isLoading: boolean;
}

/** Gives a discovered command the shape a session reports it in. */
export function discoveredCommand(command: AgentCommand): acp.AvailableCommand {
  return {
    name: command.name,
    description: command.description,
    ...(command.inputHint === undefined
      ? {}
      : { input: { hint: command.inputHint } }),
  };
}

/**
 * Selects and bounds the commands a menu shows.
 *
 * The text is whatever the agent wrote, often taken from repository files, and is shown before the
 * user has started anything. An unusable name drops its own entry and nothing else.
 */
export function presentableCommands(
  commands: readonly acp.AvailableCommand[],
): acp.AvailableCommand[] {
  const shown: acp.AvailableCommand[] = [];
  for (const command of commands) {
    if (shown.length === MAX_COMMANDS) break;
    if (!isPresentableName(command.name)) continue;
    const description = truncateBytes(
      command.description,
      MAX_DESCRIPTION_BYTES,
    );
    shown.push({
      ...command,
      description:
        description === command.description ? description : `${description}…`,
      ...(command.input == null
        ? {}
        : {
            input: {
              ...command.input,
              hint: truncateBytes(command.input.hint, MAX_HINT_BYTES),
            },
          }),
    });
  }
  return shown;
}

/** Names are typed after a slash, so one that is empty or cannot be one token is unusable. */
function isPresentableName(name: string): boolean {
  return (
    name !== "" &&
    !/[\s\p{Cc}]/u.test(name) &&
    new TextEncoder().encode(name).length <= MAX_NAME_BYTES
  );
}

/** Cuts text to a UTF-8 byte budget without splitting a character. */
function truncateBytes(text: string, limit: number): string {
  // `encodeInto` stops at the last whole character that fits.
  const { read } = new TextEncoder().encodeInto(text, new Uint8Array(limit));
  return text.slice(0, read);
}
