import { createChatStore } from "@ora/chat";
import type { ContractsClient, DeveloperModeResponse } from "@ora/contracts";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { useDeveloperMode } from "../../state/hooks/use-developer-mode";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createSettingsMemory,
  settingsHandlers,
} from "../../test/memory/settings";
import { DeveloperModeSettings } from "./developer-mode-settings";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return { ...createSettingsMemory() };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...settingsHandlers(state),
  };
}

describe("DeveloperModeSettings", () => {
  beforeEach(async () => {
    await appI18n.changeLanguage("en-US");
  });

  it("keeps the switch disabled while the authoritative value is loading", () => {
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const client = createTestClient(clientHandlers);
    clientHandlers.getDeveloperMode = vi.fn(
      () => new Promise<DeveloperModeResponse>(() => undefined),
    );

    renderSettings(client);

    expect(
      screen.getByRole("switch", { name: "Developer mode" }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("status")).toHaveTextContent(
      "Loading developer mode…",
    );
  });

  it.each(["Web", "Desktop"])(
    "persists a successful update through the %s contracts client",
    async () => {
      const user = userEvent.setup();
      const state = createFixtureState();
      const clientHandlers: TestHandlers = createFixtureHandlers(state);
      const client = createTestClient(clientHandlers);
      const setDeveloperMode = vi.spyOn(client.developerMode, "set");
      renderSettings(client);

      const toggle = await screen.findByRole("switch", {
        name: "Developer mode",
      });
      await waitFor(() => expect(toggle).toBeEnabled());
      await user.click(toggle);

      await waitFor(() =>
        expect(setDeveloperMode).toHaveBeenCalledWith({ enabled: true }),
      );
      await waitFor(() => expect(toggle).toBeChecked());
      expect(state.developerMode).toEqual({ enabled: true });
    },
  );

  it("retains the last authoritative value and prevents duplicate pending submissions", async () => {
    const user = userEvent.setup();
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const client = createTestClient(clientHandlers);
    let rejectUpdate: ((reason: Error) => void) | undefined;
    clientHandlers.setDeveloperMode = vi.fn(
      () =>
        new Promise<DeveloperModeResponse>((_resolve, reject) => {
          rejectUpdate = reject;
        }),
    );
    renderSettings(client);

    const toggle = await screen.findByRole("switch", {
      name: "Developer mode",
    });
    await waitFor(() => expect(toggle).toBeEnabled());
    await user.click(toggle);
    await waitFor(() =>
      expect(toggle).toHaveAttribute("aria-disabled", "true"),
    );
    await user.click(toggle);
    expect(clientHandlers.setDeveloperMode).toHaveBeenCalledTimes(1);

    rejectUpdate?.(new Error("persistence failed"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "last effective setting",
    );
    expect(toggle).not.toBeChecked();
  });

  it("keeps developer mode unavailable after a read failure and supports retry", async () => {
    const user = userEvent.setup();
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const client = createTestClient(clientHandlers);
    clientHandlers.getDeveloperMode = vi
      .fn()
      .mockRejectedValueOnce(new Error("read failed"))
      .mockResolvedValueOnce({ enabled: false });
    renderSettings(client);

    const toggle = screen.getByRole("switch", { name: "Developer mode" });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "could not be loaded",
    );
    expect(toggle).toHaveAttribute("aria-disabled", "true");
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(clientHandlers.getDeveloperMode).toHaveBeenCalledTimes(2);
  });
});

/** Renders the switch from the real hook so query and mutation behavior stay covered together. */
function renderSettings(client: ContractsClient) {
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(
    client,
    queryClient,
    createChatStore(client.session),
  );

  function Harness() {
    return <DeveloperModeSettings controller={useDeveloperMode()} />;
  }

  return render(<Harness />, { wrapper: Wrapper });
}
