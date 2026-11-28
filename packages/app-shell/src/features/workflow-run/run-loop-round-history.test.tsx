import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { GraphWorkflowRound } from "@ora/workflow-runtime";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { RunLoopRoundHistory } from "./run-loop-round-history";

const rounds: GraphWorkflowRound[] = [
  {
    id: "round-1",
    parentLoopNodeRunId: "loop-run",
    parentLoopNodeId: "loop",
    roundIndex: 1,
    status: "succeeded",
    nodeStates: {
      writer: { status: "succeeded", sessionId: "session-first" },
    },
    createdAt: "2026-09-16T08:00:00.000Z",
    updatedAt: "2026-09-16T08:01:00.000Z",
  },
  {
    id: "round-2",
    parentLoopNodeRunId: "loop-run",
    parentLoopNodeId: "loop",
    roundIndex: 2,
    status: "running",
    nodeStates: { reviewer: { status: "running" } },
    createdAt: "2026-09-16T08:02:00.000Z",
    updatedAt: "2026-09-16T08:03:00.000Z",
  },
];

describe("RunLoopRoundHistory", () => {
  it("distinguishes the same round number in two container attempts", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    const original = {
      ...rounds[0],
      nodeStates: {
        writer: { status: "succeeded" as const, nodeRunId: "writer-first" },
      },
    };
    const rebuilt = {
      ...original,
      id: "new-round",
      parentLoopNodeRunId: "loop-rebuilt",
      nodeStates: {
        writer: { status: "running" as const, nodeRunId: "writer-new" },
      },
    };
    render(
      <RunLoopRoundHistory
        rounds={[original, rebuilt]}
        nodeTitles={{ writer: "Writer" }}
      />,
    );
    expect(screen.getByText("writer-first")).toBeVisible();
    await user.click(
      screen.getByRole("tab", { name: /Round 1.*loop-rebuilt/ }),
    );
    expect(screen.getByText("writer-new")).toBeVisible();
    expect(screen.queryByText("writer-first")).toBeNull();
  });
  it("defaults to the first round and keeps repeated states isolated when selecting history", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <RunLoopRoundHistory
          rounds={rounds}
          nodeTitles={{ writer: "Writer", reviewer: "Reviewer" }}
        />
      </AppI18nProvider>,
    );

    expect(screen.getByText("Writer")).toBeInTheDocument();
    expect(screen.queryByText("Reviewer")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Round 2" }));

    expect(screen.getByText("Reviewer")).toBeInTheDocument();
    expect(screen.queryByTitle("session-first")).not.toBeInTheDocument();
    expect(screen.queryByText("Writer")).not.toBeInTheDocument();
  });
});
