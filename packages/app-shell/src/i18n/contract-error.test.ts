import {
  LocalTransportError,
  RemoteContractError,
  UnknownRemoteError,
  localTransportErrorKinds,
  publicErrorSchema,
} from "@ora/contracts";
import { describe, expect, it } from "vitest";
import { activeLocale, appI18n, translationResources } from "./i18n-instance";
import {
  describeChatError,
  hasDiagnosticRequestId,
  localizeContractError,
} from "./contract-error";

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

describe("plugin package failures", () => {
  it("renders the rejected field and reason instead of a bare request ID", () => {
    const error = new RemoteContractError(
      {
        code: "plugin_package_invalid",
        requestId: "00000000-0000-4000-8000-000000000004",
        params: {
          field: "description",
          message: "missing field `description`",
        },
      },
      {},
    );
    const expected =
      activeLocale() === "zh-CN"
        ? "插件包内容不合法（description）：missing field `description`"
        : "The plugin package is invalid at description: missing field `description`";

    expect(localizeContractError(error, appI18n.t)).toBe(expected);
    expect(hasDiagnosticRequestId(error)).toBe(false);
  });
});

describe("workflow dependency failures", () => {
  it("names the node and the dependency a start could not resolve", () => {
    const skill = new RemoteContractError(
      {
        code: "workflow_skill_not_found",
        requestId: "00000000-0000-4000-8000-000000000005",
        params: {
          nodeId: "reviewer",
          nodeTitle: "Code review",
          skillId: "code_review",
        },
      },
      {},
    );
    const mcp = new RemoteContractError(
      {
        code: "workflow_mcp_not_found",
        requestId: "00000000-0000-4000-8000-000000000006",
        params: {
          nodeId: "reviewer",
          nodeTitle: "Code review",
          mcpId: "local/github",
        },
      },
      {},
    );

    const ambiguous = new RemoteContractError(
      {
        code: "workflow_skill_ambiguous",
        requestId: "00000000-0000-4000-8000-000000000007",
        params: {
          nodeId: "reviewer",
          nodeTitle: "Code review",
          skillRef: "code_review",
          candidates: [
            "local-uuid (local)",
            "plugin:local/review-pack:code_review (local/review-pack)",
          ],
        },
      },
      {},
    );

    expect(localizeContractError(skill, appI18n.t)).toBe(
      activeLocale() === "zh-CN"
        ? "节点「Code review」需要的技能 code_review 不可用，请安装或恢复该技能后重试。"
        : "The skill code_review required by node 'Code review' is unavailable. Install or restore the skill, then retry.",
    );
    expect(localizeContractError(mcp, appI18n.t)).toBe(
      activeLocale() === "zh-CN"
        ? "节点「Code review」需要的 MCP local/github 不可用，请安装或修复该 MCP 插件后重试。"
        : "The MCP local/github required by node 'Code review' is unavailable. Install or repair the MCP plugin, then retry.",
    );
    expect(localizeContractError(ambiguous, appI18n.t)).toBe(
      activeLocale() === "zh-CN"
        ? "节点「Code review」引用的技能名 code_review 存在歧义，匹配到多个技能（local-uuid (local)、plugin:local/review-pack:code_review (local/review-pack)），请在节点中指定具体技能后重试。"
        : "The skill name 'code_review' referenced by node 'Code review' is ambiguous and matches multiple skills (local-uuid (local), plugin:local/review-pack:code_review (local/review-pack)). Select a specific skill for this node, then retry.",
    );
    expect(hasDiagnosticRequestId(ambiguous)).toBe(false);
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

describe("chat error descriptions", () => {
  it("translates contract failures and keeps an agent's own wording", () => {
    const refusal = new RemoteContractError(
      {
        code: "workspace_unavailable",
        requestId: "00000000-0000-4000-8000-000000000004",
        params: {},
      },
      {},
    );

    expect([
      describeChatError(refusal, appI18n.t),
      describeChatError(new Error("model quota exceeded"), appI18n.t),
    ]).toEqual([
      appI18n.t("errors.workspace_unavailable"),
      "model quota exceeded",
    ]);
    expect(describeChatError(refusal, appI18n.t)).not.toContain(
      "Remote Ora request failed",
    );
  });
});
