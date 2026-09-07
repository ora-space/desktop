import { describe, expect, it, vi } from "vitest";
import { composeTranslationResources } from "./resource-bundle";
import { featureTranslationResources, translationResources } from "./resources";

// Resource composition must remain importable without initializing either runtime integration.
vi.mock("i18next", () => {
  throw new Error("resources imported i18next");
});
vi.mock("react-i18next", () => {
  throw new Error("resources imported React i18n");
});

describe("feature-owned translation resources", () => {
  it("loads every feature independently with complete logical keys in both languages", () => {
    for (const [owner, bundle] of Object.entries(featureTranslationResources)) {
      expect(composeTranslationResources({ [owner]: bundle })).toEqual(bundle);
    }
    expect(composeTranslationResources(featureTranslationResources)).toEqual(
      translationResources,
    );
  });

  it("rejects duplicate ownership instead of letting a later feature overwrite copy", () => {
    const bundle = {
      "zh-CN": { "common.save": "保存" },
      "en-US": { "common.save": "Save" },
    };
    expect(() =>
      composeTranslationResources({ first: bundle, second: bundle }),
    ).toThrow("Duplicate translation key common.save: first and second");
  });

  it("reports the owning feature and the missing language key", () => {
    expect(() =>
      composeTranslationResources({
        chat: { "zh-CN": {}, "en-US": { "chat.send": "Send" } },
      }),
    ).toThrow(
      "Translation keys differ in chat: zh-CN missing [chat.send]; en-US missing []",
    );
  });

  it("preserves English plural variants and rejects a missing form", () => {
    const bundle = {
      "zh-CN": { "files.count": "{{count}} 个文件" },
      "en-US": {
        "files.count_one": "{{count}} file",
        "files.count_other": "{{count}} files",
      },
    };
    expect(composeTranslationResources({ files: bundle })).toEqual(bundle);
    expect(() =>
      composeTranslationResources({
        files: {
          ...bundle,
          "en-US": { "files.count_one": "{{count}} file" },
        },
      }),
    ).toThrow("Missing en-US plural form: files.count_other");
  });
});
