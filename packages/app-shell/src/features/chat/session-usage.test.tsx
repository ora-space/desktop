import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { SessionUsage } from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { beforeEach, describe, expect, it } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { SessionUsageIndicator } from "./session-usage";

beforeEach(async () => {
  await act(() => appI18n.changeLanguage("en-US"));
});

function renderUsage(usage: SessionUsage) {
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <SessionUsageIndicator usage={usage} />
      </TooltipProvider>,
    ),
  };
}

describe("SessionUsageIndicator", () => {
  it("explains why a reloaded historical session has no usage", async () => {
    const { user } = renderUsage({
      context: { status: "needs_interaction" },
      lastTurnTokens: { status: "none" },
    });

    await user.click(
      screen.getByRole("button", {
        name: /not saved with conversation history/i,
      }),
    );

    expect(
      screen.getByRole("heading", { name: "Current context usage" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /Send a message and, if the current agent supports ACP/i,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Previous-turn token usage" }),
    ).toBeInTheDocument();
  });

  it("shows raw counters, an exact stack, and click-accessible relationship details", async () => {
    const { user } = renderUsage({
      context: {
        status: "reported",
        snapshot: {
          usedTokens: 34_000,
          sizeTokens: 100_000,
          cost: { amount: 0.42, currency: "USD" },
          receivedAt: Date.now(),
        },
      },
      lastTurnTokens: {
        status: "reported",
        receivedAt: Date.now(),
        usage: {
          accountingScope: "unspecified",
          totalTokens: 100_000n,
          inputTokens: 40_000n,
          outputTokens: 15_000n,
          thoughtTokens: 10_000n,
          cachedReadTokens: 30_000n,
          cachedWriteTokens: 5_000n,
        },
      },
    });

    await user.click(screen.getByRole("button", { name: /Context 34%/i }));

    const tokenSection = screen.getByRole("region", {
      name: "Previous-turn token usage",
    });
    expect(
      within(tokenSection).getByLabelText("Previous-turn token composition"),
    ).toBeInTheDocument();
    expect(
      within(tokenSection).getByText(
        /Total 100,000 = Input 40,000 \+ Output 15,000/,
      ),
    ).toBeInTheDocument();
    expect(within(tokenSection).getByText("Cache write")).toBeInTheDocument();

    await user.click(
      within(tokenSection).getByRole("button", {
        name: /token data attached to the previous response/i,
      }),
    );
    expect(
      within(tokenSection).getByText(
        /A missing field means unreported, not zero/i,
      ),
    ).toBeInTheDocument();
  });

  it("hides a misleading stack when categories overlap", async () => {
    const { user } = renderUsage({
      context: { status: "unavailable" },
      lastTurnTokens: {
        status: "reported",
        receivedAt: Date.now(),
        usage: {
          accountingScope: "unspecified",
          totalTokens: 100n,
          inputTokens: 90n,
          outputTokens: 20n,
        },
      },
    });

    await user.click(
      screen.getByRole("button", {
        name: /still did not report context usage/i,
      }),
    );

    expect(
      screen.queryByLabelText("Previous-turn token composition"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/Reported categories total 110/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Cache read")).not.toBeInTheDocument();
  });

  it("renders unclassified tokens when known categories leave a gap", async () => {
    const { user } = renderUsage({
      context: { status: "unavailable" },
      lastTurnTokens: {
        status: "reported",
        receivedAt: Date.now(),
        usage: {
          accountingScope: "unspecified",
          totalTokens: 100n,
          inputTokens: 40n,
          outputTokens: 15n,
        },
      },
    });

    await user.click(
      screen.getByRole("button", {
        name: /still did not report context usage/i,
      }),
    );

    expect(
      screen.getByLabelText("Previous-turn token composition"),
    ).toBeInTheDocument();
    expect(screen.getAllByText(/unclassified 45/i)).toHaveLength(2);
    expect(
      screen.getByText(
        /Total 100 = reported categories 55 \+ unclassified 45/i,
      ),
    ).toBeInTheDocument();
  });

  it("keeps zero-total fields visible without drawing a stack", async () => {
    const { user } = renderUsage({
      context: { status: "unavailable" },
      lastTurnTokens: {
        status: "reported",
        receivedAt: Date.now(),
        usage: {
          accountingScope: "unspecified",
          totalTokens: 0n,
          inputTokens: 0n,
          outputTokens: 0n,
        },
      },
    });

    await user.click(
      screen.getByRole("button", {
        name: /still did not report context usage/i,
      }),
    );

    expect(
      screen.queryByLabelText("Previous-turn token composition"),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/reported a total of zero/i)).toBeInTheDocument();
    expect(screen.getByText("Input")).toBeInTheDocument();
    expect(screen.getByText("Output")).toBeInTheDocument();
  });

  it("uses the warning and danger ring thresholds without changing raw values", () => {
    const warning = renderUsage({
      context: {
        status: "reported",
        snapshot: { usedTokens: 80, sizeTokens: 100, receivedAt: Date.now() },
      },
      lastTurnTokens: { status: "none" },
    });
    expect(
      warning
        .getByRole("button", { name: /Context 80%/i })
        .querySelector(".text-amber-500"),
    ).toBeInTheDocument();
    warning.unmount();

    renderUsage({
      context: {
        status: "reported",
        snapshot: { usedTokens: 120, sizeTokens: 100, receivedAt: Date.now() },
      },
      lastTurnTokens: { status: "none" },
    });
    expect(
      screen
        .getByRole("button", { name: /Context 100% · 120 \/ 100/i })
        .querySelector(".text-destructive"),
    ).toBeInTheDocument();
  });

  it("exposes the global reliability disclaimer on hover and click", async () => {
    const { user } = renderUsage({
      context: { status: "unavailable" },
      lastTurnTokens: { status: "unavailable" },
    });
    const help = screen.getByRole("button", { name: "About usage statistics" });

    await user.hover(help);
    expect(
      await screen.findByText(/Usage statistics are for reference only/i),
    ).toBeInTheDocument();
    await user.click(help);
    expect(
      screen.getByRole("heading", { name: "Current context usage" }),
    ).toBeInTheDocument();
  });
});
