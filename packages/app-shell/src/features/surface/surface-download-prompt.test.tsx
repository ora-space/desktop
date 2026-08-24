import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ContractsClient } from "@ora/contracts";
import { ContractsClientContext } from "../../contracts-client-context";
import { AppI18nProvider } from "../../i18n/i18n";
import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { SurfaceDownloadPrompt } from "./surface-download-prompt";

/** A prepared import session with no candidates; enough for the review dialog to open. */
const SESSION = {
  sessionId: "session-1",
  status: "prepared" as const,
  candidates: [],
  progress: { results: [] },
};

/** Contracts client stub serving exactly the prepared session above. */
function createClient() {
  return {
    skillImport: {
      get: vi.fn(async () => ({ session: SESSION })),
      cancel: vi.fn(async () => ({})),
    },
  } as unknown as ContractsClient;
}

function choiceEvent(downloadId: number, fileName: string) {
  return {
    type: "downloadChoice" as const,
    instance: 1,
    pluginId: "official/acme.hub",
    downloadId,
    pageOrigin: "https://www.example.com",
    fileName,
    sizeBytes: 2048,
    actions: ["import_skill" as const, "save_as" as const],
  };
}

function renderPrompt(platform: PlatformAdapter, client: ContractsClient) {
  return render(
    <PlatformProvider adapter={platform}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>
          <SurfaceDownloadPrompt />
        </AppI18nProvider>
      </ContractsClientContext.Provider>
    </PlatformProvider>,
  );
}

beforeEach(() => {
  useSurfaceStore.setState({
    embeddedSupported: null,
    records: {},
    failures: {},
    sidePanelInstance: null,
    downloadPrompts: [],
  });
});

describe("SurfaceDownloadPrompt", () => {
  it("resolves import_skill and opens the skill-import review", async () => {
    const user = userEvent.setup();
    const host = createSurfaceTestPlatform({ embedded: false });
    host.surfaces.resolveDownload.mockResolvedValue({
      action: "import_skill",
      importSessionId: "session-1",
    });
    const client = createClient();
    renderPrompt(host.platform, client);

    act(() =>
      useSurfaceStore.getState().applyEvent(choiceEvent(5, "pack.zip")),
    );
    expect(screen.getByText(/pack\.zip/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "导入为技能" }));

    await waitFor(() =>
      expect(screen.getByText("导入技能")).toBeInTheDocument(),
    );
    expect({
      resolved: host.surfaces.resolveDownload.mock.calls,
      fetched: (client.skillImport.get as ReturnType<typeof vi.fn>).mock.calls,
      prompts: useSurfaceStore.getState().downloadPrompts,
    }).toEqual({
      resolved: [[5, "import_skill"]],
      fetched: [[{ sessionId: "session-1" }]],
      prompts: [],
    });
  });

  it("keeps the prompt open when the save dialog is cancelled, then saves", async () => {
    const user = userEvent.setup();
    const host = createSurfaceTestPlatform({ embedded: false });
    const selectSavePath = vi
      .fn()
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce("/home/user/pack.zip");
    const platform = { ...host.platform, selectSavePath };
    renderPrompt(platform, createClient());

    act(() =>
      useSurfaceStore.getState().applyEvent(choiceEvent(6, "pack.zip")),
    );

    // The cancelled native dialog leaves the prompt open and resolves nothing.
    await user.click(screen.getByRole("button", { name: "另存为…" }));
    expect(host.surfaces.resolveDownload).not.toHaveBeenCalled();
    expect(screen.getByText(/pack\.zip/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "另存为…" }));
    await waitFor(() =>
      expect(useSurfaceStore.getState().downloadPrompts).toEqual([]),
    );
    expect(host.surfaces.resolveDownload.mock.calls).toEqual([
      [6, "save_as", "/home/user/pack.zip"],
    ]);
  });

  it("discards a dismissed download and shows queued prompts one at a time", async () => {
    const user = userEvent.setup();
    const host = createSurfaceTestPlatform({ embedded: false });
    renderPrompt(host.platform, createClient());

    act(() => {
      useSurfaceStore.getState().applyEvent(choiceEvent(7, "first.zip"));
      useSurfaceStore.getState().applyEvent(choiceEvent(8, "second.zip"));
    });
    expect(screen.getByText(/first\.zip/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "忽略" }));

    await waitFor(() =>
      expect(screen.getByText(/second\.zip/)).toBeInTheDocument(),
    );
    expect(host.surfaces.discardDownload.mock.calls).toEqual([[7]]);
  });

  it("opens the import review when an automatic import completes", async () => {
    const host = createSurfaceTestPlatform({ embedded: false });
    const client = createClient();
    renderPrompt(host.platform, client);
    // Let the event subscription settle before emitting.
    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      host.emit({
        type: "downloadCompleted",
        instance: 1,
        pluginId: "official/acme.hub",
        downloadId: 9,
        fileName: "skill.zip",
        action: "import_skill",
        importSessionId: "session-1",
      });
      await Promise.resolve();
    });

    await waitFor(() =>
      expect(screen.getByText("导入技能")).toBeInTheDocument(),
    );
    expect(
      (client.skillImport.get as ReturnType<typeof vi.fn>).mock.calls,
    ).toEqual([[{ sessionId: "session-1" }]]);
  });
});
