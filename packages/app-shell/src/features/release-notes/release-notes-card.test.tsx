import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { PlatformProvider } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import type { ReleaseNote } from "./bundled-release-notes";
import { ReleaseNotesCard } from "./release-notes-card";
import { ReleaseNotesHistory } from "./release-notes-history";

const RECORD_KEY = "ora.releaseNotes.promptedVersion";
const MARKUP = '<img src="x"> **bold** [link](https://example.invalid)';
const notes: ReleaseNote[] = [
  {
    version: "0.7.0",
    date: "2026-12-10",
    entries: [
      { category: "feature", "zh-CN": "新功能七", "en-US": "Feature seven" },
    ],
  },
  {
    version: "0.8.0",
    date: "2027-01-10",
    entries: [{ category: "important", "zh-CN": MARKUP, "en-US": MARKUP }],
  },
];

/** Mounts the card under the providers the shell supplies. */
function renderCard(
  version: string,
  isNew: (() => Promise<boolean>) | undefined = () => Promise.resolve(false),
) {
  const answer = vi.fn(isNew);
  return {
    answer,
    ...render(
      <AppI18nProvider>
        <PlatformProvider
          adapter={{
            ...createStubPlatform(),
            installation: { isNew: answer },
          }}
        >
          <ReleaseNotesCard align="right" notes={notes} version={version} />
        </PlatformProvider>
      </AppI18nProvider>,
    ),
  };
}

beforeEach(async () => {
  window.localStorage.clear();
  await appI18n.changeLanguage("zh-CN");
});

describe("ReleaseNotesCard", () => {
  it("prompts unseen versions once and stays closed after dismissal", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem(RECORD_KEY, "0.6.0");
    const first = renderCard("0.8.0");

    const card = await screen.findByRole("complementary", {
      name: "已更新到 Ora 0.8.0",
    });
    expect(
      within(card)
        .getAllByRole("heading", { level: 3 })
        .map((heading) => heading.textContent),
    ).toEqual(["0.8.0", "0.7.0"]);
    expect(window.localStorage.getItem(RECORD_KEY)).toBe("0.8.0");
    await user.click(screen.getByRole("button", { name: "关闭更新日志" }));
    expect(screen.queryByRole("complementary")).toBeNull();
    first.unmount();

    const second = renderCard("0.8.0");
    await waitFor(() => expect(second.answer).toHaveBeenCalled());
    await second.answer.mock.results[0].value;
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("remembers a fresh installation without prompting", async () => {
    renderCard("0.8.0", () => Promise.resolve(true));
    await waitFor(() =>
      expect(window.localStorage.getItem(RECORD_KEY)).toBe("0.8.0"),
    );
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("prompts an upgrade from a version that kept no record", async () => {
    renderCard("0.7.0");
    expect(await screen.findByText("新功能七")).not.toBeNull();
  });

  it("leaves the record alone when the host cannot answer", async () => {
    const { answer } = renderCard("0.8.0", () =>
      Promise.reject(new Error("unavailable")),
    );
    await expect(answer.mock.results[0].value).rejects.toThrow("unavailable");
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(window.localStorage.getItem(RECORD_KEY)).toBeNull();
  });

  it("follows the interface language", async () => {
    await appI18n.changeLanguage("en-US");
    renderCard("0.7.0");
    expect(await screen.findByText("Feature seven")).not.toBeNull();
    expect(screen.getByText("New")).not.toBeNull();
  });

  it("groups a version by category, with what needs attention first", async () => {
    render(
      <AppI18nProvider>
        <ReleaseNotesHistory
          version="0.9.0"
          notes={[
            {
              version: "0.9.0",
              date: "2027-02-10",
              entries: (
                ["fix", "feature", "important", "improvement", "fix"] as const
              ).map((category, index) => ({
                category,
                "zh-CN": `条目 ${index}`,
                "en-US": `Entry ${index}`,
              })),
            },
          ]}
        />
      </AppI18nProvider>,
    );
    expect(
      screen
        .getAllByRole("heading", { level: 4 })
        .map((heading) => heading.textContent),
    ).toEqual(["重要", "新功能", "改进", "修复"]);
    expect(
      screen.getAllByRole("listitem").map((item) => item.textContent),
    ).toEqual(["条目 2", "条目 1", "条目 3", "条目 0", "条目 4"]);
  });

  it("renders markup as plain text", async () => {
    const { container } = renderCard("0.8.0");
    expect(await screen.findByText(MARKUP)).not.toBeNull();
    expect(container.querySelector("img, a, strong")).toBeNull();
  });
});

describe("ReleaseNotesHistory", () => {
  it("never shows a version newer than the running one", () => {
    render(
      <AppI18nProvider>
        <ReleaseNotesHistory notes={notes} version="0.7.0" />
      </AppI18nProvider>,
    );
    expect(screen.getByText("新功能七")).not.toBeNull();
    expect(screen.queryByText(MARKUP)).toBeNull();
  });

  it("shows an empty state for a build without notes", () => {
    render(
      <AppI18nProvider>
        <ReleaseNotesHistory />
      </AppI18nProvider>,
    );
    expect(screen.getByText("这个版本没有更新日志。")).not.toBeNull();
  });
});
