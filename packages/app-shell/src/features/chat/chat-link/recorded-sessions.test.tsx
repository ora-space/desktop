import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import type { ChatTurn } from "@ora/chat";
import { loadSessionConversation } from "@ora/chat";
import type { LoadSessionEvent } from "@ora/contracts";
import { ContractsClientContext } from "../../../contracts-client-context";
import { AppI18nProvider } from "../../../i18n/i18n";
import { appI18n } from "../../../i18n/i18n-instance";
import { PlatformProvider } from "../../../platform";
import {
  createMockClient,
  createMockClientState,
} from "../../../test/mock-client";
import { createStubPlatform } from "../../../test/stub-platform";
import { TaskChangesNavigationProvider } from "../../diff/task-changes-navigation";
import { MessageList } from "../message-list";
import powershellNamePsIsContainer from "./fixtures/powershell-name-psiscontainer.json";
import powershellRecursiveRelative from "./fixtures/powershell-recursive-relative.json";

/**
 * Navigation entry point a recorded token must use when clicked:
 * `files` previews a file, `artifact` resolves the entry's kind from its parent
 * listing first, and `none` stays plain text (an ambiguous name that would open
 * the wrong path).
 */
type ExpectedSurface = "files" | "artifact" | "none";

interface RecordedCase {
  /** Name shown in the test title; matches the fixture file. */
  fixture: string;
  /** Recorded session log lines, replayed through the real session loader. */
  records: RecordedLine[];
  /** Checkout the session ran in; links resolve against it exactly as in the app. */
  workspaceRoot: string;
  tokens: Record<string, ExpectedSurface>;
}

/**
 * Cases are recorded agent turns, not hand-built payloads: an agent picks the
 * command, and its exact output shape is what the linker has to survive.
 */
const RECORDED_CASES: RecordedCase[] = [
  {
    fixture: "powershell-name-psiscontainer.json",
    records: powershellNamePsIsContainer as unknown as RecordedLine[],
    workspaceRoot: "C:/home/codebase/acp-test",
    tokens: {
      __pycache__: "artifact",
      ".claude": "artifact",
      ".opencode": "artifact",
      ".venv": "artifact",
      docs: "artifact",
      openspec: "artifact",
      ".gitignore": "files",
      ".python-version": "files",
      "agents.py": "files",
      "main.py": "files",
      "pyproject.toml": "files",
      "README.md": "files",
      "uv.lock": "files",
    },
  },
  {
    // A recursive listing printing workspace-relative paths, rendered back as a
    // tree fence: entries carry no kind column, so every hit resolves its kind
    // from the parent listing on click.
    fixture: "powershell-recursive-relative.json",
    records: powershellRecursiveRelative as unknown as RecordedLine[],
    workspaceRoot: "C:/home/codebase/acp-test",
    tokens: {
      ".claude": "artifact",
      ".opencode": "artifact",
      __pycache__: "artifact",
      docs: "artifact",
      openspec: "artifact",
      "docs/superpowers": "artifact",
      ".claude/commands/opsx": "artifact",
      "main.py": "artifact",
      "uv.lock": "artifact",
      // `commands`, `skills` and every `openspec-*` skill exist under both
      // `.claude` and `.opencode`; linking one would open the wrong path.
      commands: "none",
      skills: "none",
      "openspec-apply-change": "none",
    },
  },
];

/** One recorded line as Ora's session log writes it. */
type RecordedLine =
  | {
      type: "update";
      update: Extract<LoadSessionEvent, { type: "session_update" }>["update"];
    }
  | {
      type: "turnEnded";
      stop_reason: Extract<
        LoadSessionEvent,
        { type: "turn_ended" }
      >["stopReason"];
    };

/**
 * Replays a recorded transcript through the same loader the app uses when a
 * session is reopened, so the turns under test are built by production code
 * rather than by a fixture author's idea of the payload.
 */
async function recordedTurns(records: RecordedLine[]): Promise<ChatTurn[]> {
  const events: LoadSessionEvent[] = records.map((record) =>
    record.type === "turnEnded"
      ? { type: "turn_ended", stopReason: record.stop_reason }
      : { type: "session_update", update: record.update },
  );
  const client = createMockClient(createMockClientState()).session;
  const conversation = await loadSessionConversation(
    {
      ...client,
      load: async function* () {
        for (const event of events) yield event;
        yield { type: "completed" as const };
      },
    },
    "recorded-session",
  );
  return conversation.turns;
}

/** Lets `resolveTaskCwd` settle so the clean-stderr gate stays quiet. */
async function flushDesktopCwd() {
  await act(async () => {
    await Promise.resolve();
  });
}

async function renderRecordedSession(recorded: RecordedCase) {
  const turns = await recordedTurns(recorded.records);
  const openWorkspaceFile = vi.fn();
  const openWorkspaceDirectory = vi.fn();
  const openWorkspaceArtifact = vi.fn();
  const mockClient = createMockClient(createMockClientState());
  mockClient.task.getWorkspace = vi.fn(async () => ({
    workspace: { rootPath: recorded.workspaceRoot, branchName: "main" },
  }));
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <ContractsClientContext.Provider value={mockClient}>
        <PlatformProvider adapter={createStubPlatform()}>
          <AppI18nProvider>
            <TaskChangesNavigationProvider
              onOpenDiff={vi.fn()}
              onOpenWorkspaceFile={openWorkspaceFile}
              onOpenWorkspaceDirectory={openWorkspaceDirectory}
              onOpenWorkspaceArtifact={openWorkspaceArtifact}
            >
              <MessageList
                taskId="task-1"
                turns={turns}
                userName="Ada"
                isResponding={false}
              />
            </TaskChangesNavigationProvider>
          </AppI18nProvider>
        </PlatformProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
  await flushDesktopCwd();
  return { openWorkspaceFile, openWorkspaceDirectory, openWorkspaceArtifact };
}

describe("recorded chat sessions", () => {
  for (const recorded of RECORDED_CASES) {
    it(`links every recorded artifact in ${recorded.fixture}`, async () => {
      const user = userEvent.setup();
      const navigation = await renderRecordedSession(recorded);

      const opened: Record<string, ExpectedSurface> = {};
      // Links classify against the checkout cwd, which the message list resolves
      // through a query: without waiting, every token still reads as plain text.
      const ready = Object.entries(recorded.tokens).find(
        ([, surface]) => surface !== "none",
      );
      if (ready !== undefined) {
        await screen.findAllByRole("button", {
          name: appI18n.t(
            ready[1] === "files"
              ? "chat.fileLink.aria"
              : "chat.fileLink.pathAria",
            { path: ready[0] },
          ),
        });
      }

      for (const token of Object.keys(recorded.tokens)) {
        // A file link and a kind-resolving link differ by accessible name, so
        // both are tried before a token is called plain text.
        // A token can appear more than once in one message (tree plus prose);
        // every occurrence classifies the same way, so the first one stands in.
        const link =
          screen
            .queryAllByRole("button", {
              name: appI18n.t("chat.fileLink.aria", { path: token }),
            })
            .at(0) ??
          screen
            .queryAllByRole("button", {
              name: appI18n.t("chat.fileLink.pathAria", { path: token }),
            })
            .at(0);
        if (link === undefined) {
          opened[token] = "none";
          continue;
        }
        await user.click(link);
        opened[token] =
          navigation.openWorkspaceArtifact.mock.calls.length > 0
            ? "artifact"
            : "files";
        navigation.openWorkspaceArtifact.mockClear();
        navigation.openWorkspaceFile.mockClear();
      }

      expect(opened).toEqual(recorded.tokens);
      // Directories resolve their kind from the parent listing, so the legacy
      // directory-only entry point must stay unused.
      expect(navigation.openWorkspaceDirectory).not.toHaveBeenCalled();
    });
  }
});
