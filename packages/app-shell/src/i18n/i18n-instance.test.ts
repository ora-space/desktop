import { afterEach, describe, expect, it, vi } from "vitest";
import { activeLocale, appI18n } from "./i18n-instance";

afterEach(async () => {
  vi.restoreAllMocks();
  await appI18n.changeLanguage("zh-CN");
  localStorage.removeItem("ora.locale");
});

describe("single synchronous application i18n instance", () => {
  it("has feature copy available before rendering or asynchronous registration", () => {
    expect(appI18n.isInitialized).toBe(true);
    expect(appI18n.t("common.cancel", { lng: "zh-CN" })).toBe("取消");
    expect(appI18n.t("common.cancel", { lng: "en-US" })).toBe("Cancel");
  });

  it("switches locale, document language, storage, and plural resolution together", async () => {
    await appI18n.changeLanguage("en-US");
    expect([
      activeLocale(),
      document.documentElement.lang,
      localStorage.getItem("ora.locale"),
    ]).toEqual(["en-US", "en-US", "en-US"]);
    expect(appI18n.t("chat.activity.metric.files", { count: 1 })).toBe(
      "1 file",
    );
    expect(appI18n.t("chat.activity.metric.files", { count: 2 })).toBe(
      "2 files",
    );
    await appI18n.changeLanguage("zh-CN");
    expect(appI18n.t("chat.activity.metric.files", { count: 2 })).toBe(
      "2 个文件",
    );
  });

  it("keeps switching usable when persistent storage throws", async () => {
    vi.spyOn(window.localStorage, "setItem").mockImplementation(() => {
      throw new Error("storage blocked");
    });
    await appI18n.changeLanguage("en-US");
    expect([activeLocale(), appI18n.t("common.cancel")]).toEqual([
      "en-US",
      "Cancel",
    ]);
  });
});
