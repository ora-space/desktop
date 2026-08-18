import { afterEach, describe, expect, it } from "vitest";
import { appI18n, translationResources } from "./i18n-instance";
import {
  localizeSkillImportReason,
  localizeSkillImportResultReason,
  localizeSkillImportStatus,
} from "./skill-import-reason";

afterEach(async () => {
  await appI18n.changeLanguage("zh-CN");
});

describe("skill import copy", () => {
  it("keeps every import status and reason key in both locales", () => {
    const prefix = "settings.skills.import";
    const zhKeys = Object.keys(translationResources["zh-CN"]).filter((key) =>
      key.startsWith(prefix),
    );
    const enKeys = Object.keys(translationResources["en-US"]).filter((key) =>
      key.startsWith(prefix),
    );

    expect(zhKeys.sort()).toEqual(enKeys.sort());
    const zh = translationResources["zh-CN"] as Record<string, string>;
    const en = translationResources["en-US"] as Record<string, string>;
    for (const key of zhKeys) {
      expect(zh[key]?.length ?? 0).toBeGreaterThan(0);
      expect(en[key]?.length ?? 0).toBeGreaterThan(0);
    }
  });

  it("localizes importing and failed states in the active language", async () => {
    const t = appI18n.t.bind(appI18n);

    await appI18n.changeLanguage("zh-CN");
    expect(localizeSkillImportStatus("committing", t)).toBe("导入中");
    expect(localizeSkillImportStatus("failed", t)).toBe("导入失败");
    expect(localizeSkillImportReason("skill_storage_error", t)).toBe(
      "无法写入技能文件。",
    );
    expect(localizeSkillImportReason("not_a_real_code", t)).toBe("导入失败。");
    expect(
      localizeSkillImportResultReason({ status: "failed", errorCode: null }, t),
    ).toBe("导入失败。");

    await appI18n.changeLanguage("en-US");
    expect(localizeSkillImportStatus("committing", t)).toBe("Importing");
    expect(localizeSkillImportStatus("failed", t)).toBe("Import failed");
    expect(localizeSkillImportReason("skill_storage_error", t)).toBe(
      "The skill files could not be written.",
    );
    expect(localizeSkillImportReason("not_a_real_code", t)).toBe(
      "Import failed.",
    );
    expect(
      localizeSkillImportResultReason({ status: "failed", errorCode: null }, t),
    ).toBe("Import failed.");
    expect(localizeSkillImportStatus("committing", t)).not.toBe("导入中");
    expect(localizeSkillImportReason("skill_storage_error", t)).not.toBe(
      "无法写入技能文件。",
    );
  });

  it("does not leak raw English status enums", async () => {
    await appI18n.changeLanguage("zh-CN");
    expect(
      localizeSkillImportStatus("committing", appI18n.t.bind(appI18n)),
    ).not.toBe("committing");
    expect(localizeSkillImportStatus("mystery", appI18n.t.bind(appI18n))).toBe(
      "未知状态",
    );
  });
});
