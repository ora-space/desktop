import { toast } from "@ora/ui";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import { DiagnosticLogsSettings } from "./diagnostic-logs-settings";

describe("DiagnosticLogsSettings", () => {
  beforeEach(async () => {
    await appI18n.changeLanguage("en-US");
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders nothing when the host cannot export diagnostic logs", () => {
    const { container } = renderSettings(createStubPlatform());

    expect(container).toBeEmptyDOMElement();
  });

  it("downloads today's log through the shared host capability and confirms", async () => {
    const user = userEvent.setup();
    const successToast = vi
      .spyOn(toast, "success")
      .mockImplementation(() => "id");
    let finishDownload: ((downloaded: boolean) => void) | undefined;
    const downloadToday = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          finishDownload = resolve;
        }),
    );
    renderSettings({
      ...createStubPlatform(),
      diagnosticLogs: { downloadToday },
    });

    await user.click(screen.getByRole("button", { name: "Download logs" }));

    expect(downloadToday).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Downloading…" })).toBeDisabled();
    // Resolve inside act so the hook's pending-state reset stays within the test boundary.
    await act(async () => {
      finishDownload?.(true);
    });
    expect(successToast).toHaveBeenCalledWith("Today's logs were downloaded.");
    expect(screen.getByRole("button", { name: "Download logs" })).toBeEnabled();
  });

  it("stays quiet when the save dialog is dismissed", async () => {
    const user = userEvent.setup();
    const successToast = vi
      .spyOn(toast, "success")
      .mockImplementation(() => "id");
    const errorToast = vi.spyOn(toast, "error").mockImplementation(() => "id");
    renderSettings({
      ...createStubPlatform(),
      diagnosticLogs: { downloadToday: async () => false },
    });

    await user.click(screen.getByRole("button", { name: "Download logs" }));

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Download logs" }),
      ).toBeEnabled(),
    );
    expect(successToast).not.toHaveBeenCalled();
    expect(errorToast).not.toHaveBeenCalled();
  });

  it("reports a failed export with the shared error toast", async () => {
    const user = userEvent.setup();
    const errorToast = vi.spyOn(toast, "error").mockImplementation(() => "id");
    renderSettings({
      ...createStubPlatform(),
      diagnosticLogs: {
        downloadToday: async () => {
          throw new Error("copy failed");
        },
      },
    });

    await user.click(screen.getByRole("button", { name: "Download logs" }));

    await waitFor(() =>
      expect(errorToast).toHaveBeenCalledWith(
        "Could not download logs. Try again later.",
      ),
    );
    expect(screen.getByRole("button", { name: "Download logs" })).toBeEnabled();
  });
});

/** Renders the section against an explicit platform so capability gating is exercised directly. */
function renderSettings(platform: PlatformAdapter) {
  return render(
    <PlatformProvider adapter={platform}>
      <DiagnosticLogsSettings />
    </PlatformProvider>,
  );
}
