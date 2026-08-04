import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { toast } from "@ora/ui";
import {
  PlatformProvider,
  type SkillMarketplaceCapability,
  type SkillMarketplaceStatus,
} from "@ora/platform";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { SkillMarketplacePanel } from "./skill-marketplace-panel";

/** Renders the marketplace panel with one explicitly injected host capability. */
function renderMarketplace(skillMarketplace: SkillMarketplaceCapability) {
  const platform = { ...createStubPlatform(), skillMarketplace };
  return render(
    <AppI18nProvider>
      <PlatformProvider adapter={platform}>
        <SkillMarketplacePanel />
      </PlatformProvider>
    </AppI18nProvider>,
  );
}

describe("SkillMarketplacePanel", () => {
  it("opens SkillHub and displays native download progress and the final absolute path", async () => {
    const user = userEvent.setup();
    const open = vi.fn().mockResolvedValue(undefined);
    const stop = vi.fn();
    let listener: ((status: SkillMarketplaceStatus) => void) | undefined;
    const onStatus = vi.fn(async (nextListener: (status: SkillMarketplaceStatus) => void) => {
      listener = nextListener;
      return stop;
    });
    const successToast = vi.spyOn(toast, "success").mockImplementation(() => "skill-download");
    const view = renderMarketplace({ kind: "supported", open, onStatus });

    await user.click(screen.getByRole("button", { name: /打开技能市场|Open marketplace/ }));
    await waitFor(() => expect(onStatus).toHaveBeenCalledOnce());
    expect(open).toHaveBeenCalledOnce();

    act(() => listener?.({ status: "downloading", fileName: "skill.zip" }));
    expect(screen.getByRole("status")).toHaveTextContent(/正在下载 skill.zip|Downloading skill.zip/);

    act(() => listener?.({
      status: "downloaded",
      fileName: "skill.zip",
      archivePath: "/app-data/skill-downloads/skill.zip",
    }));
    expect(screen.getByRole("status")).toHaveTextContent("/app-data/skill-downloads/skill.zip");
    expect(successToast).toHaveBeenCalledWith(
      expect.stringMatching(/已下载 skill.zip|Downloaded skill.zip/),
      {
        description: "/app-data/skill-downloads/skill.zip",
        duration: 5_000,
      },
    );

    view.unmount();
    expect(stop).toHaveBeenCalledOnce();
    successToast.mockRestore();
  });

  it("shows native download failures without reporting a completed archive", async () => {
    let listener: ((status: SkillMarketplaceStatus) => void) | undefined;
    renderMarketplace({
      kind: "supported",
      open: vi.fn().mockResolvedValue(undefined),
      onStatus: async (nextListener) => {
        listener = nextListener;
        return () => {};
      },
    });
    await waitFor(() => expect(listener).toBeDefined());

    act(() => listener?.({
      status: "failed",
      stage: "download",
      code: "skill_download_cancelled",
      message: "cancelled",
    }));

    expect(screen.getByRole("alert")).toHaveTextContent(/下载失败|Download failed/);
    expect(screen.queryByText(/Saved to|保存位置/)).not.toBeInTheDocument();
  });

  it("keeps the marketplace action disabled on unsupported hosts", () => {
    renderMarketplace({ kind: "unsupported" });

    expect(screen.getByRole("button", { name: /打开技能市场|Open marketplace/ })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent(/仅在 Ora 桌面端可用|Ora Desktop only/);
  });
});
