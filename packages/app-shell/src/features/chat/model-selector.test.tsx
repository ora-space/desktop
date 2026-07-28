import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  DEFAULT_SETTINGS,
  useSettingsStore,
} from "../../state/stores/settings-store";
import { ModelSelector } from "./model-selector";

beforeEach(() => {
  window.localStorage.clear();
  useSettingsStore.setState({ settings: { ...DEFAULT_SETTINGS } });
});

describe("ModelSelector", () => {
  it("shows a bounded coding-agent catalog with one agent expanded", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <ModelSelector />
      </AppI18nProvider>,
    );

    await user.click(screen.getByRole("button", { name: /选择模型|Select model/ }));

    const catalog = screen
      .getByText(/Agent 模型|Agent models/)
      .closest('[data-slot="popover-content"]');
    expect(catalog).toHaveClass(
      "max-h-[min(24rem,var(--available-height))]",
      "overflow-y-auto",
      "w-72",
    );
    expect(
      screen.getByText(/4 个 Agent|4 Agents/),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: /^OpenCode/, expanded: true }),
    ).toBeVisible();
    for (const agentName of ["Codex", "Claude Code", "CodeAgent"]) {
      expect(
        screen.getByRole("button", {
          name: new RegExp(`^${agentName}`),
          expanded: false,
        }),
      ).toBeVisible();
    }
  });

  it("keeps mock selection local and preserves the OpenCode runtime settings", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <ModelSelector />
      </AppI18nProvider>,
    );

    await user.click(screen.getByRole("button", { name: /选择模型|Select model/ }));
    await user.click(screen.getByRole("button", { name: /^Claude Code/ }));
    await user.click(
      screen.getByRole("button", { name: /Claude Sonnet 5/ }),
    );

    expect(useSettingsStore.getState().settings).toEqual(DEFAULT_SETTINGS);
    const trigger = screen.getByRole("button", {
      name: /选择模型|Select model/,
    });
    expect(trigger).toHaveTextContent("Claude Sonnet 5");
    expect(trigger).not.toHaveTextContent("OpenCode");
    expect(
      trigger.querySelector("[data-coding-agent]"),
    ).toHaveAttribute("data-coding-agent", "claude_code");
  });

  it("clears mock ids persisted by the previous coupled implementation", async () => {
    useSettingsStore.setState({
      settings: {
        ...DEFAULT_SETTINGS,
        model: "claude-code/claude-sonnet-5",
      },
    });

    render(
      <AppI18nProvider>
        <ModelSelector />
      </AppI18nProvider>,
    );

    await waitFor(() => {
      expect(useSettingsStore.getState().settings).toEqual(DEFAULT_SETTINGS);
    });
    expect(
      screen.getByRole("button", { name: /选择模型|Select model/ }),
    ).toHaveTextContent("DeepSeek V4 Pro");
  });
});
