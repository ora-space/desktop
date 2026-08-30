import { createElement } from "react";
import { createChatStore } from "@ora/chat";
import type { McpApplicationStateDto } from "@ora/contracts";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import type { McpApplicationStateController } from "../../state/hooks/use-mcp-application-state";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { McpSettings } from "./mcp-settings";

describe("McpSettings", () => {
  beforeEach(async () => {
    await appI18n.changeLanguage("en-US");
  });

  it.each<{ state: McpApplicationStateDto; label: string }>([
    { state: "needs_configuration", label: "Needs configuration" },
    { state: "waiting_for_agent", label: "Waiting for agent" },
    { state: "applying", label: "Applying" },
    { state: "ready", label: "Ready" },
    { state: "failed", label: "Failed" },
  ])("renders the $label state label", ({ state, label }) => {
    renderSettings(makeController({ state }));

    expect(screen.getByText(label)).toBeInTheDocument();
  });

  it("shows the loading indicator while the state is loading", () => {
    renderSettings(makeController({ isLoading: true, state: undefined }));

    expect(screen.getByRole("status")).toHaveTextContent("Loading");
  });

  it("prompts to open a workspace when none is active", () => {
    renderSettings(makeController({ workspaceId: null, state: undefined }));

    expect(
      screen.getByText("Open a workspace to view its MCP state."),
    ).toBeInTheDocument();
  });

  it("shows a load error and retries on demand", async () => {
    const user = userEvent.setup();
    const refetch = vi.fn();
    renderSettings(
      makeController({ error: new Error("boom"), state: undefined, refetch }),
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not read MCP state.",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(refetch).toHaveBeenCalledTimes(1);
  });
});

/** Builds a controller with resting defaults so each case overrides only what it asserts on. */
function makeController(
  overrides: Partial<McpApplicationStateController> = {},
): McpApplicationStateController {
  return {
    workspaceId: "ws-1",
    state: undefined,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
    ...overrides,
  };
}

/** Renders the panel through the shared providers so `useTranslation` resolves real keys. */
function renderSettings(controller: McpApplicationStateController) {
  const client = createMockClient(createMockClientState());
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(
    client,
    queryClient,
    createChatStore(client.session),
  );

  return render(createElement(McpSettings, { controller }), {
    wrapper: Wrapper,
  });
}
