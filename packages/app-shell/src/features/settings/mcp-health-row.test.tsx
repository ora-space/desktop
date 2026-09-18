import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  ContractsClient,
  McpHealthEntry,
  McpHealthErrorCode,
} from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { describe, expect, it } from "vitest";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createPluginMemory,
  pluginHandlers,
  seedMcpHealth,
} from "../../test/memory/plugins";
import "../../i18n/i18n-instance";
import { useMcpHealth } from "../../state/hooks/use-mcp-health";
import { McpHealthRow } from "./mcp-health-row";

const PLUGIN_ID = "official/tavily";

/** The card view's identity: no Session cwd is bound. */
function entry(status: McpHealthEntry["status"]): McpHealthEntry {
  return {
    identity: {
      pluginId: PLUGIN_ID,
      packageVersion: "1.0.0",
      configurationRevision: 1,
      transport: "http",
      cwd: null,
    },
    status,
  };
}

/**
 * Marks the point at which the card health query answered, so negative assertions do not race the
 * first fetch.
 */
function CardHealthSettled() {
  const health = useMcpHealth(null);
  if (!health.isSuccess) return null;
  return <span data-testid="card-health-settled" />;
}

/** Renders the card row over a mock backend seeded with one card-view result. */
function renderRow(options: {
  seeded?: McpHealthEntry[];
  probeResult?: McpHealthEntry | null;
}) {
  const state = createPluginMemory();
  seedMcpHealth(state, null, options.seeded ?? []);
  state.probeMcpHealthResult = options.probeResult;
  const client: ContractsClient = createTestClient(
    pluginHandlers(state) as TestHandlers,
  );
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  render(
    <Wrapper>
      <McpHealthRow pluginId={PLUGIN_ID} />
      <CardHealthSettled />
    </Wrapper>,
  );
}

describe("McpHealthRow", () => {
  it("shows the closed code for a server the Host cannot reach", async () => {
    renderRow({
      seeded: [
        entry({ status: "unhealthy", error_code: "mcp_http_unauthorized" }),
      ],
    });

    const row = await screen.findByText(/mcp_http_unauthorized|HTTP 鉴权失败/);
    expect(row).toBeDefined();
    const slot = document.querySelector("[data-slot='mcp-health']");
    expect(slot).toHaveAttribute("data-mcp-health", "unhealthy");
    expect(slot).toHaveAttribute(
      "data-mcp-health-code",
      "mcp_http_unauthorized",
    );
    // Host health never claims the session took effect, and never uses the retired wording.
    expect(slot).not.toHaveTextContent(/MCP Ready/);
  });

  it("re-detects on demand and presents the fresh result", async () => {
    const user = userEvent.setup();
    renderRow({
      seeded: [entry({ status: "unhealthy", error_code: "mcp_probe_timeout" })],
      probeResult: entry({ status: "healthy" }),
    });

    await screen.findByRole("button", { name: /重新检测|Re-detect/ });
    await user.click(
      screen.getByRole("button", { name: /重新检测|Re-detect/ }),
    );

    const slot = document.querySelector("[data-slot='mcp-health']");
    await screen.findByText(/Host 已完成握手|Host completed the handshake/);
    expect(slot).toHaveAttribute("data-mcp-health", "healthy");
    // The success wording states the boundary explicitly: Host handshake, not session effect.
    expect(slot).toHaveTextContent(
      /不代表该 MCP 已在会话内生效|does not mean this MCP is in effect inside a session/,
    );
  });

  it("keeps a workspace-context member unexplained rather than faking a cwd", async () => {
    renderRow({
      seeded: [entry({ status: "unknown", reason: "context_missing" })],
    });

    await screen.findByTestId("card-health-settled");
    const slot = document.querySelector("[data-slot='mcp-health']");
    expect(slot).toHaveAttribute("data-mcp-health", "unknown");
    // No re-detect action exists: it could only run against an invented workspace directory.
    expect(screen.queryByRole("button")).toBeNull();
    expect(slot).toHaveTextContent(
      /需要会话的工作目录|Needs a session workspace directory/,
    );
  });

  it("renders nothing when the plugin has no health entry at all", async () => {
    renderRow({ seeded: [] });

    await screen.findByTestId("card-health-settled");
    expect(document.querySelector("[data-slot='mcp-health']")).toBeNull();
  });

  // One row per closed code: the presentation must name every probe outcome without inventing a
  // new code or falling back to a raw server string.
  const unhealthyCodes: [McpHealthErrorCode, RegExp][] = [
    ["mcp_spawn_failed", /无法启动 MCP 进程|The MCP process could not start/],
    [
      "mcp_exited_prematurely",
      /进程在握手完成前退出|exited before the handshake/,
    ],
    ["mcp_handshake_failed", /握手失败|The handshake failed/],
    ["mcp_probe_timeout", /检测超时|The probe timed out/],
    ["mcp_tools_unavailable", /无法读取工具列表|Tools could not be listed/],
    ["mcp_http_unreachable", /无法连接 HTTP 端点|HTTP endpoint is unreachable/],
    ["mcp_http_unauthorized", /HTTP 鉴权失败|HTTP authentication failed/],
    [
      "mcp_http_server_error",
      /HTTP 服务端错误|HTTP endpoint returned an error/,
    ],
  ];

  it.each(unhealthyCodes)(
    "presents %s with its stable code and localized label",
    async (code, label) => {
      renderRow({ seeded: [entry({ status: "unhealthy", error_code: code })] });

      await screen.findByText(label);
      const slot = document.querySelector("[data-slot='mcp-health']");
      expect(slot).toHaveAttribute("data-mcp-health", "unhealthy");
      expect(slot).toHaveAttribute("data-mcp-health-code", code);
      // The wording is Host-observed, never an in-session promise.
      expect(slot).toHaveTextContent(
        /Host 当前连不上|The Host cannot reach it right now/,
      );
    },
  );

  it("presents the not-probed state with a re-detect action", async () => {
    renderRow({ seeded: [entry({ status: "unknown", reason: "not_probed" })] });

    await screen.findByText(/尚未检测|Not probed yet/);
    expect(document.querySelector("[data-slot='mcp-health']")).toHaveAttribute(
      "data-mcp-health",
      "unknown",
    );
    expect(
      screen.getByRole("button", { name: /重新检测|Re-detect/ }),
    ).toBeDefined();
  });

  it("presents a healthy result as a Host handshake, not session effect", async () => {
    renderRow({ seeded: [entry({ status: "healthy" })] });

    await screen.findByText(/Host 已完成握手|Host completed the handshake/);
    const slot = document.querySelector("[data-slot='mcp-health']");
    expect(slot).toHaveAttribute("data-mcp-health", "healthy");
    expect(slot).toHaveTextContent(
      /不代表该 MCP 已在会话内生效|does not mean this MCP is in effect inside a session/,
    );
  });
});
