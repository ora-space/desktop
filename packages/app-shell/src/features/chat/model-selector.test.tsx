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
  it("shows a bounded provider catalog with one provider expanded", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <ModelSelector />
      </AppI18nProvider>,
    );

    await user.click(screen.getByRole("button", { name: /选择模型|Select model/ }));

    const catalog = screen
      .getByText(/模型目录|Model catalog/)
      .closest('[data-slot="popover-content"]');
    expect(catalog).toHaveClass(
      "max-h-[min(20rem,var(--available-height))]",
      "overflow-y-auto",
    );
    expect(
      screen.getByRole("button", { name: /^DeepSeek/, expanded: true }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: /^Claude/, expanded: false }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: /^OpenAI/ })).toBeVisible();
    expect(screen.getByRole("button", { name: /^OpenCode/ })).toBeVisible();
  });

  it("keeps mock selection local and preserves the OpenCode runtime settings", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <ModelSelector />
      </AppI18nProvider>,
    );

    await user.click(screen.getByRole("button", { name: /选择模型|Select model/ }));
    await user.click(screen.getByRole("button", { name: /^Claude/ }));
    await user.click(
      screen.getByRole("button", { name: /Claude Sonnet 4\.5/ }),
    );

    expect(useSettingsStore.getState().settings).toEqual(DEFAULT_SETTINGS);
    expect(
      screen.getByRole("button", { name: /选择模型|Select model/ }),
    ).toHaveTextContent("Claude Sonnet 4.5");
  });

  it("clears mock ids persisted by the previous coupled implementation", async () => {
    useSettingsStore.setState({
      settings: {
        ...DEFAULT_SETTINGS,
        model: "anthropic/claude-sonnet-4.5",
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
