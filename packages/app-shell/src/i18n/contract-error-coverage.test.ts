import { describe, expect, it } from "vitest";
import { publicErrorSchema } from "@ora/contracts";
import { translationResources } from "./resources";
import type { Locale } from "./resource-bundle";

/**
 * Reads every public error code out of the generated contract schema.
 *
 * The union is regenerated from the Rust `PublicError` enum, so deriving the catalog here keeps a
 * backend variant from reaching the frontend without anyone maintaining a second hand-written list.
 */
function contractErrorCodes(): string[] {
  return publicErrorSchema.options.map((option) => option.shape.code.value);
}

const locales: Locale[] = ["zh-CN", "en-US"];

describe("contract error translation coverage", () => {
  // `localizeContractError` falls back to `errors.unknown` for an unmapped code, which reads as
  // "quote this request ID" — the copy reserved for defects the user cannot act on. A new backend
  // error silently inherits that text, so the gap has to fail here rather than in front of a user.
  it("localizes every public error code in every supported language", () => {
    const codes = contractErrorCodes();
    const missing = locales.flatMap((locale) =>
      codes
        .filter((code) => !(`errors.${code}` in translationResources[locale]))
        .map((code) => `${locale}: errors.${code}`),
    );

    expect(missing).toEqual([]);
  });

  it("derives a non-empty catalog so a broken schema shape cannot pass as full coverage", () => {
    expect(contractErrorCodes()).toContain("internal_error");
  });
});
