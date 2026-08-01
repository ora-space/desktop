import { localTransportErrorKinds, publicErrorSchema } from "@ora/contracts";
import { describe, expect, it } from "vitest";
import { translationResources } from "./i18n-instance";

const interpolationFields = (text: string): string[] =>
  [...text.matchAll(/{{(\w+)}}/g)].map((match) => match[1]).sort();

describe("contract error translations", () => {
  it("covers every generated public code in Chinese and English with valid interpolation", () => {
    for (const option of publicErrorSchema.options) {
      const code = option.shape.code.value;
      const key = `errors.${code}` as keyof (typeof translationResources)["zh-CN"];
      const zh = translationResources["zh-CN"][key];
      const en = translationResources["en-US"][key];
      const paramsShape = (
        option.shape.params as unknown as {
          shape?: Record<string, unknown>;
        }
      ).shape;
      const allowedFields = new Set([
        ...Object.keys(paramsShape ?? {}),
        "requestId",
      ]);

      expect(zh, `missing zh-CN translation for ${code}`).toBeTypeOf("string");
      expect(en, `missing en-US translation for ${code}`).toBeTypeOf("string");
      expect(interpolationFields(zh)).toEqual(interpolationFields(en));
      expect(interpolationFields(zh).every((field) => allowedFields.has(field))).toBe(true);
    }
  });

  it("covers unknown remote and every finite local transport failure", () => {
    expect(translationResources["zh-CN"]["errors.unknown"]).toBeTypeOf("string");
    expect(translationResources["en-US"]["errors.unknown"]).toBeTypeOf("string");

    for (const kind of localTransportErrorKinds) {
      const key = `errors.transport.${kind}` as keyof (typeof translationResources)["zh-CN"];
      expect(translationResources["zh-CN"][key]).toBeTypeOf("string");
      expect(translationResources["en-US"][key]).toBeTypeOf("string");
    }
  });
});
