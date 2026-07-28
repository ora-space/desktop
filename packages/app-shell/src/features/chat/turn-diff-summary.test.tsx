import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { ChatToolCall, ChatTurn } from "@ora/chat";
import { AppI18nProvider } from "../../i18n/i18n";
import { collectTurnDiffFiles } from "./turn-diff-files";
import { TurnDiffSummary } from "./turn-diff-summary";

/** Creates one completed file-edit tool without involving the ACP transport. */
function editTool(
  id: string,
  path: string,
  oldText: string,
  newText: string,
): ChatToolCall {
  return {
    kind: "toolCall",
    id,
    title: `Edit ${path}`,
    toolKind: "edit",
    status: "completed",
    content: [{ type: "diff", path, oldText, newText }],
    locations: [{ path }],
    createdAt: 10,
    updatedAt: 20,
  };
}

/** Creates one response turn with a stable user message for component tests. */
function turn(items: ChatToolCall[], status: ChatTurn["status"] = "completed"): ChatTurn {
  return {
    id: "turn-1",
    userMessage: {
      kind: "message",
      id: "user-1",
      role: "user",
      content: "Make the change",
      createdAt: 1,
    },
    items,
    status,
    stopReason: null,
    error: null,
    createdAt: 1,
  };
}

describe("turn diff summary", () => {
  it("merges repeated edits and reports the final per-file line totals", () => {
    expect(collectTurnDiffFiles(turn([
      editTool("edit-1", "src/main.ts", "const value = 1;\n", "const value = 2;\n"),
      editTool("edit-2", "src/main.ts", "const value = 2;\n", "const value = 3;\n"),
      editTool("edit-3", "src/new.ts", "", "export {};\n"),
    ]))).toEqual([
      {
        path: "src/main.ts",
        oldText: "const value = 1;\n",
        newText: "const value = 3;\n",
        additions: 1,
        deletions: 1,
      },
      {
        path: "src/new.ts",
        oldText: "",
        newText: "export {};\n",
        additions: 1,
        deletions: 0,
      },
    ]);
  });

  it("opens the selected file in the diff viewer", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <TurnDiffSummary
          turn={turn([
            editTool("edit-1", "src/main.ts", "const value = 1;\n", "const value = 2;\n"),
          ])}
        />
      </AppI18nProvider>,
    );

    await user.click(screen.getByRole("button", { name: /src\/main\.ts/ }));

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "src/main.ts" })).toBeInTheDocument();
    expect(screen.getByText("const value = 2;")).toBeInTheDocument();
  });

  it("waits for turn completion before showing the summary", () => {
    render(
      <AppI18nProvider>
        <TurnDiffSummary
          turn={turn([
            editTool("edit-1", "src/main.ts", "", "export {};\n"),
          ], "streaming")}
        />
      </AppI18nProvider>,
    );

    expect(screen.queryByRole("button", { name: /src\/main\.ts/ })).not.toBeInTheDocument();
  });
});
