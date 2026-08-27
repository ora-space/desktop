import { createChatStore } from "@ora/chat";
import type { ContractsClient } from "@ora/contracts";
import { PlatformProvider } from "../../platform";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { useUiStore } from "../../state/stores/ui-store";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { createStubPlatform } from "../../test/stub-platform";
import { SettingsDialog } from "./settings-dialog";

describe("SettingsDialog developer options", () => {
  beforeEach(async () => {
    await appI18n.changeLanguage("en-US");
    useUiStore.setState({ settingsOpen: true });
  });

  it("keeps Developer options reachable and reveals log level in place after enabling developer mode", async () => {
    const client = createMockClient(createMockClientState());
    renderDialog(client);

    expect(
      screen.getByRole("button", { name: "Developer options" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Advanced" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Developer options" }),
    );
    const developerModeSwitch = await screen.findByRole("switch", {
      name: "Developer mode",
    });
    expect(developerModeSwitch).toBeEnabled();
    expect(
      screen.queryByRole("combobox", { name: "Log level" }),
    ).not.toBeInTheDocument();
    await userEvent.click(developerModeSwitch);
    expect(
      await screen.findByRole("combobox", { name: "Log level" }),
    ).toBeInTheDocument();
  });

  it("keeps Developer options reachable and hides log level when the initial read fails", async () => {
    const client = createMockClient(createMockClientState());
    client.developerMode.get = vi
      .fn()
      .mockRejectedValue(new Error("read failed"));
    renderDialog(client);

    expect(
      screen.getByRole("button", { name: "Developer options" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Advanced" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Developer options" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "could not be loaded",
    );
    expect(
      screen.getByRole("switch", { name: "Developer mode" }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.queryByRole("combobox", { name: "Log level" }),
    ).not.toBeInTheDocument();
  });

  it("shows the switch and authoritative effective log level together when enabled", async () => {
    const state = createMockClientState();
    state.developerMode = { enabled: true };
    state.runtimeLogLevel = {
      configuredLevel: "info",
      effectiveLevel: "trace",
      startupOverride: "trace",
    };
    renderDialog(createMockClient(state));

    const developerNavigation = await screen.findByRole("button", {
      name: "Developer options",
    });
    await userEvent.click(developerNavigation);

    expect(
      await screen.findByRole("switch", { name: "Developer mode" }),
    ).toBeChecked();
    const selector = await screen.findByRole("combobox", { name: "Log level" });
    expect(selector).toHaveTextContent("Trace (most detailed)");
    expect(screen.queryByText(/ORA_LOG_LEVEL/)).not.toBeInTheDocument();
  });

  it("stays on Developer options and hides log level after developer mode is disabled", async () => {
    const state = createMockClientState();
    state.developerMode = { enabled: true };
    renderDialog(createMockClient(state));

    await userEvent.click(
      await screen.findByRole("button", { name: "Developer options" }),
    );
    expect(
      await screen.findByRole("combobox", { name: "Log level" }),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("switch", { name: "Developer mode" }),
    );
    await waitFor(() => {
      expect(
        screen.queryByRole("combobox", { name: "Log level" }),
      ).not.toBeInTheDocument();
    });
    expect(
      screen.getByRole("heading", { name: "Developer options" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("switch", { name: "Developer mode" }),
    ).not.toBeChecked();
    expect(
      screen.getByRole("button", { name: "Developer options" }),
    ).toBeInTheDocument();
  });

  it("protects unsaved plugin configuration when switching settings categories", async () => {
    const state = createMockClientState();
    state.installedPlugins.push({
      id: "official/weather",
      namespace: "official",
      name: "weather",
      displayName: "weather",
      version: "1.2.0",
      description: "Weather plugin",
      homepage: null,
      license: null,
      kind: "agent",
      agentDisplayName: "weather",
      logo: null,
      installationValidity: { validity: "valid" },
      configuration: { state: "available", completeness: "incomplete" },
      runtime: "stopped",
    });
    state.pluginConfigurations.set("official/weather", {
      pluginId: "official/weather",
      schemaVersion: 1,
      revision: 0n,
      declarationFingerprint: "declaration-1",
      settings: [
        {
          declaration: {
            id: "endpoint",
            title: "Endpoint",
            description: "Service URL",
            type: "string",
            required: true,
            order: null,
            default: null,
          },
          storedValue: null,
          effectiveValue: null,
          source: "absent",
          valueErrorCode: null,
        },
      ],
      summary: { state: "available", completeness: "incomplete" },
    });
    const user = userEvent.setup();
    renderDialog(createMockClient(state));

    await user.click(screen.getByRole("button", { name: "Plugins" }));
    await user.click(
      await screen.findByRole("button", { name: "Manage plugins" }),
    );
    await user.click(await screen.findByRole("button", { name: "Configure" }));
    await user.type(await screen.findByLabelText("Endpoint"), "https://api");
    await user.click(screen.getByRole("button", { name: "Appearance" }));

    expect(
      await screen.findByRole("alertdialog", {
        name: "Save configuration changes?",
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Appearance" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Discard" }));
    expect(
      await screen.findByRole("heading", { name: "Appearance" }),
    ).toBeInTheDocument();
  });
});

/** Renders the real settings dialog with shared client, query, chat, i18n, and platform providers. */
function renderDialog(client: ContractsClient) {
  const queryClient = createTestQueryClient();
  const AppProviders = createHookWrapper(
    client,
    queryClient,
    createChatStore(client.session),
  );

  function Wrapper({ children }: { children: ReactNode }) {
    return (
      <PlatformProvider adapter={createStubPlatform()}>
        <AppProviders>{children}</AppProviders>
      </PlatformProvider>
    );
  }

  return { ...render(<SettingsDialog />, { wrapper: Wrapper }), queryClient };
}
