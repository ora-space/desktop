import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  PlatformProvider,
  type LocationActionsCapability,
  type SkillMarketplaceCapability,
  type SkillMarketplaceStatus,
} from "../../platform";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { SkillMarketplacePanel } from "./skill-marketplace-panel";

/** Renders the marketplace panel with one explicitly injected host capability. */
function renderMarketplace(
  skillMarketplace: SkillMarketplaceCapability,
  locationActions: LocationActionsCapability = {
    resolveTaskCwd: vi.fn(),
    open: vi.fn(),
  },
) {
  const platform = {
    ...createStubPlatform(),
    skillMarketplace,
    locationActions,
  };
  return render(
    <AppI18nProvider>
      <PlatformProvider adapter={platform}>
        <SkillMarketplacePanel />
      </PlatformProvider>
    </AppI18nProvider>,
  );
}

describe("SkillMarketplacePanel", () => {
  it("opens SkillHub and reveals the completed archive directory on request", async () => {
    const user = userEvent.setup();
    const open = vi.fn().mockResolvedValue(undefined);
    const stop = vi.fn();
    let listener: ((status: SkillMarketplaceStatus) => void) | undefined;
    const onStatus = vi.fn(
      async (nextListener: (status: SkillMarketplaceStatus) => void) => {
        listener = nextListener;
        return stop;
      },
    );
    const openLocation = vi.fn().mockResolvedValue(undefined);
    const view = renderMarketplace(
      { open, onStatus },
      {
        resolveTaskCwd: vi.fn(),
        open: openLocation,
      },
    );

    await user.click(
      screen.getByRole("button", { name: /打开技能市场|Open marketplace/ }),
    );
    await waitFor(() => expect(onStatus).toHaveBeenCalledOnce());
    expect(open).toHaveBeenCalledWith("skillHub");

    act(() =>
      listener?.({
        status: "downloading",
        provider: "skillHub",
        fileName: "skill.zip",
      }),
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      /正在下载 skill.zip|Downloading skill.zip/,
    );

    act(() =>
      listener?.({
        status: "downloaded",
        provider: "skillHub",
        fileName: "skill.zip",
        archivePath: "/app-data/skill-downloads/skill.zip",
      }),
    );
    expect(screen.getByRole("status")).not.toHaveTextContent(
      /已下载 skill.zip|Downloaded skill.zip/,
    );
    expect(
      screen.queryByText("/app-data/skill-downloads/skill.zip"),
    ).not.toBeInTheDocument();
    const savedLocation = screen.getByRole("button", {
      name: /保存位置|Saved to/,
    });
    expect(savedLocation).toHaveAttribute(
      "title",
      "/app-data/skill-downloads/skill.zip",
    );
    await user.click(savedLocation);
    expect(openLocation).toHaveBeenCalledWith(
      "explorer",
      "/app-data/skill-downloads",
    );

    view.unmount();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("shows native download failures without reporting a completed archive", async () => {
    let listener: ((status: SkillMarketplaceStatus) => void) | undefined;
    renderMarketplace({
      open: vi.fn().mockResolvedValue(undefined),
      onStatus: async (nextListener) => {
        listener = nextListener;
        return () => {};
      },
    });
    await waitFor(() => expect(listener).toBeDefined());

    act(() =>
      listener?.({
        status: "failed",
        provider: "skillHub",
        stage: "download",
        code: "skill_download_cancelled",
        message: "cancelled",
      }),
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      /下载失败|Download failed/,
    );
    expect(screen.queryByText(/Saved to|保存位置/)).not.toBeInTheDocument();
  });

  it("opens the Huawei provider directly without an intermediate dialog", async () => {
    const user = userEvent.setup();
    const open = vi.fn().mockResolvedValue(undefined);
    renderMarketplace({
      open,
      onStatus: async () => () => {},
    });

    await user.click(
      screen.getByRole("button", {
        name: /打开内网 Skill Market|Open internal Skill Market/,
      }),
    );

    expect(open).toHaveBeenCalledWith("huaweiAgentCenter");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
