import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import { type SkillImportSession } from "@ora/contracts";
import { PlatformProvider, type SkillMarketplaceStatus } from "../../platform";
import { toast } from "@ora/ui";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { createStubPlatform } from "../../test/stub-platform";
import { SkillMarketplaceInstallController } from "./skill-marketplace-install-controller";

/** Builds one prepared archive session with the requested candidate status. */
function preparedSession(status: "ready" | "conflict"): SkillImportSession {
  return {
    sessionId: "marketplace-import-1",
    status: "prepared",
    createdAt: 1n,
    candidates: [
      {
        candidateId: "candidate-1",
        name: "review-skill",
        description: "Reviews changes",
        sourcePath: "review-skill/SKILL.md",
        fileCount: 1,
        totalSize: 128n,
        status,
        errorCode: null,
        existingSkill:
          status === "conflict"
            ? {
                skillId: "skill-1",
                updatedAt: 2n,
                description: "Existing review skill",
              }
            : null,
      },
    ],
    progress: { total: 1, processed: 0, results: [] },
  };
}

/** Projects a completed successful result from one prepared session. */
function completedSession(
  session: SkillImportSession,
  result: "imported" | "overwritten",
): SkillImportSession {
  return {
    ...session,
    status: "completed",
    progress: {
      total: 1,
      processed: 1,
      results: [
        {
          candidateId: "candidate-1",
          name: "review-skill",
          status: result,
          errorCode: null,
        },
      ],
    },
  };
}

/** Renders the process-wide installer and exposes the native marketplace status listener. */
function renderController(
  session: SkillImportSession,
  result: "imported" | "overwritten",
) {
  const state = createMockClientState();
  const client = createMockClient(state);
  client.skillImport.prepare = vi.fn(async () => ({ session }));
  client.skillImport.commit = vi.fn(async () => ({
    sessionId: session.sessionId,
    status: "committing" as const,
    progress: session.progress,
  }));
  client.skillImport.get = vi.fn(async () => ({
    session: completedSession(session, result),
  }));
  let listener: ((status: SkillMarketplaceStatus) => void) | undefined;
  const platform = {
    ...createStubPlatform(),
    skillMarketplace: {
      open: vi.fn(async () => undefined),
      onStatus: vi.fn(
        async (nextListener: (status: SkillMarketplaceStatus) => void) => {
          listener = nextListener;
          return () => {};
        },
      ),
    },
  };
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  render(
    <Wrapper>
      <PlatformProvider adapter={platform}>
        <SkillMarketplaceInstallController />
      </PlatformProvider>
    </Wrapper>,
  );
  return { client, getListener: () => listener };
}

/** Emits the archive completion payload produced by the native WebView downloader. */
function emitDownloaded(
  listener: ((status: SkillMarketplaceStatus) => void) | undefined,
) {
  act(() =>
    listener?.({
      status: "downloaded",
      provider: "skillHub",
      fileName: "review-skill.zip",
      archivePath: "C:\\Ora\\skill-downloads\\review-skill.zip",
    }),
  );
}

describe("SkillMarketplaceInstallController", () => {
  it("automatically commits a downloaded archive whose candidates are ready", async () => {
    const session = preparedSession("ready");
    const { client, getListener } = renderController(session, "imported");
    const successToast = vi
      .spyOn(toast, "success")
      .mockImplementation(() => "installed");
    await waitFor(() => expect(getListener()).toBeDefined());

    emitDownloaded(getListener());
    emitDownloaded(getListener());

    await waitFor(() =>
      expect(client.skillImport.prepare).toHaveBeenCalledWith({
        source: {
          kind: "archive",
          path: "C:\\Ora\\skill-downloads\\review-skill.zip",
          fileName: "review-skill.zip",
        },
      }),
    );
    expect(client.skillImport.prepare).toHaveBeenCalledOnce();
    await waitFor(() =>
      expect(client.skillImport.commit).toHaveBeenCalledWith({
        sessionId: session.sessionId,
        decisions: [],
      }),
    );
    await waitFor(
      () =>
        expect(successToast).toHaveBeenCalledWith("已自动安装 1 个 Skill。"),
      {
        timeout: 2_000,
      },
    );
    successToast.mockRestore();
  });

  it("reuses the import dialog before overwriting an existing skill", async () => {
    const user = userEvent.setup();
    const session = preparedSession("conflict");
    const { client, getListener } = renderController(session, "overwritten");
    await waitFor(() => expect(getListener()).toBeDefined());

    emitDownloaded(getListener());

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("Existing review skill", { exact: false }),
    ).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "覆盖" }));
    await user.click(within(dialog).getByRole("button", { name: "确认导入" }));

    await waitFor(() =>
      expect(client.skillImport.commit).toHaveBeenCalledWith({
        sessionId: session.sessionId,
        decisions: [{ candidateId: "candidate-1", decision: "overwrite" }],
      }),
    );
    await waitFor(
      () => expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
      {
        timeout: 4_000,
      },
    );
  });
});
