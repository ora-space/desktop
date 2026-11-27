import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { OriginTag } from "./origin-tag";

void appI18n;

describe("OriginTag", () => {
  it("names the official marketplace and explains it on hover", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <OriginTag origin={{ kind: "official" }} />
      </AppI18nProvider>,
    );

    const tag = screen.getByText(/^(官方|Official)$/);
    await user.hover(tag);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      /Ora 官方插件市场，只读，随插件更新|Ora's official plugin marketplace/,
    );
  });

  it("shows a marketplace label without leaking its address into the tooltip", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <OriginTag
          origin={{ kind: "marketplace", label: "obsismc/marketplace" }}
        />
      </AppI18nProvider>,
    );

    await user.hover(screen.getByText("obsismc/marketplace"));
    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).toHaveTextContent(
      /第三方插件市场，首次从该市场引入|Third-party plugin marketplace/,
    );
    expect(tooltip).not.toHaveTextContent(/obsismc|github/);
  });
});
