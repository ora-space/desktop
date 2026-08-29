import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { MarkdownDocument } from "./markdown-message";

/** Renders a sent prompt through the same compact surface user bubbles use. */
function renderPrompt(content: string) {
  return render(
    <AppI18nProvider>
      <MarkdownDocument density="compact" content={content} />
    </AppI18nProvider>,
  );
}

describe("compact user-prompt chips", () => {
  it("re-renders a sent skill token as the chip the composer showed", () => {
    renderPrompt("review $code-review now");

    const chip = document.querySelector("[data-prompt-token='skill']");
    expect(chip).not.toBeNull();
    expect(chip).toHaveClass("composer-mention");
    expect(chip?.textContent).toBe("$code-review");
    expect(screen.queryByText(/review now/)).toBeInTheDocument();
  });

  it("re-renders a sent command token as a chip", () => {
    renderPrompt("run /test please");
    const chip = document.querySelector("[data-prompt-token='command']");
    expect(chip).not.toBeNull();
    expect(chip).toHaveClass("composer-mention");
    expect(chip?.textContent).toBe("/test");
  });

  it("re-renders a sent role token as a chip", () => {
    renderPrompt("ask @designer");
    const chip = document.querySelector("[data-prompt-token='role']");
    expect(chip).not.toBeNull();
    expect(chip).toHaveClass("composer-mention");
    expect(chip?.textContent).toBe("@designer");
  });

  it("keeps tokens glued to a word or an email handle as plain text", () => {
    renderPrompt("cost$x and user@example.com");
    expect(document.querySelector("[data-prompt-token]")).toBeNull();
    expect(screen.getByText(/cost\$x/)).toBeInTheDocument();
    expect(screen.getByText(/user@example\.com/)).toBeInTheDocument();
  });

  it("keeps a multi-segment slash path as plain text, not command chips", () => {
    renderPrompt("edit /usr/bin/parser");
    expect(document.querySelector("[data-prompt-token]")).toBeNull();
    expect(screen.getByText(/\/usr\/bin\/parser/)).toBeInTheDocument();
  });
});
