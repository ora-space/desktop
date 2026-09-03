import {
  LocalTransportError,
  RemoteContractError,
  UnknownRemoteError,
  localTransportErrorKinds,
  publicErrorSchema,
} from "@ora/contracts";
import { describe, expect, it } from "vitest";
import { translationResources } from "./i18n-instance";
import { hasDiagnosticRequestId } from "./contract-error";

const interpolationFields = (text: string): string[] =>
  [...text.matchAll(/{{(\w+)}}/g)].map((match) => match[1]).sort();

describe("contract error translations", () => {
  it("covers every generated public code in Chinese and English with valid interpolation", () => {
    for (const option of publicErrorSchema.options) {
      const code = option.shape.code.value;
      const key = `errors.${code}`;
      const zh = (
        translationResources["zh-CN"] as Record<string, string | undefined>
      )[key];
      const en = (
        translationResources["en-US"] as Record<string, string | undefined>
      )[key];
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
      if (typeof zh !== "string" || typeof en !== "string") continue;
      expect(interpolationFields(zh)).toEqual(interpolationFields(en));
      expect(
        interpolationFields(zh).every((field) => allowedFields.has(field)),
      ).toBe(true);
    }
  });

  it("covers unknown remote and every finite local transport failure", () => {
    expect(translationResources["zh-CN"]["errors.unknown"]).toBeTypeOf(
      "string",
    );
    expect(translationResources["en-US"]["errors.unknown"]).toBeTypeOf(
      "string",
    );

    for (const kind of localTransportErrorKinds) {
      const key = `errors.transport.${kind}`;
      expect(
        (translationResources["zh-CN"] as Record<string, string | undefined>)[
          key
        ],
      ).toBeTypeOf("string");
      expect(
        (translationResources["en-US"] as Record<string, string | undefined>)[
          key
        ],
      ).toBeTypeOf("string");
    }
  });
});

describe("diagnostic request IDs", () => {
  it("offers logs only for remote failures whose message asks for a request ID", () => {
    const internal = new RemoteContractError(
      {
        code: "internal_error",
        requestId: "00000000-0000-4000-8000-000000000001",
        params: {},
      },
      {},
    );
    const invalid = new RemoteContractError(
      {
        code: "invalid_request",
        requestId: "00000000-0000-4000-8000-000000000002",
        params: {},
      },
      {},
    );
    const unknown = new UnknownRemoteError(
      "future_error",
      "00000000-0000-4000-8000-000000000003",
      {},
    );

    expect([
      hasDiagnosticRequestId(internal),
      hasDiagnosticRequestId(invalid),
      hasDiagnosticRequestId(unknown),
      hasDiagnosticRequestId(
        new LocalTransportError("tauri_invoke_failure", "failed"),
      ),
    ]).toEqual([true, false, true, false]);
  });
});
