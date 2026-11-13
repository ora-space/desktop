import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createChatStore } from "@ora/chat";
import type {
  GetSessionBindingsResponse,
  SessionBindings,
} from "@ora/contracts";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import { ChatStoreContext } from "../../chat-store-context";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { SessionInformation, BindingTimeline } from "./session-information";

beforeEach(async () => {
  await act(() => appI18n.changeLanguage("en-US"));
});

const binding = (
  sessionId: string,
  id = "opaque/full-session-id",
): SessionBindings => ({
  type: "available",
  sessionId,
  history: "readable",
  segments: [
    {
      key: "history:0",
      agentRef: "org.agent",
      agentSessionId: id,
      recordedAt: "2026-10-22T10:00:00+08:00",
      current: false,
    },
    {
      key: "history:2",
      agentRef: "org.agent",
      agentSessionId: id,
      recordedAt: "2026-10-22T11:00:00+08:00",
      current: true,
    },
  ],
});

function fixture(handlers: TestHandlers) {
  const client = createTestClient(handlers);
  const store = createChatStore(client.session);
  const cache = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Surface({ id }: { id?: string }) {
    return (
      <QueryClientProvider client={cache}>
        <ContractsClientContext value={client}>
          <ChatStoreContext value={store}>
            <SessionInformation key={id} sessionId={id} />
          </ChatStoreContext>
        </ContractsClientContext>
      </QueryClientProvider>
    );
  };
}

describe("session information", () => {
  it("discards a closed query even when reopened for the same session", async () => {
    const user = userEvent.setup();
    const requests: Array<{
      signal?: AbortSignal;
      resolve: (value: GetSessionBindingsResponse) => void;
    }> = [];
    const Surface = fixture({
      getSessionBindings: (_, options) =>
        new Promise((resolve) => {
          requests.push({ signal: options?.signal, resolve });
        }),
    });
    render(<Surface id="same-session" />);
    const open = () =>
      user.click(
        screen.getByRole("button", {
          name: appI18n.t("chat.sessionInformation"),
        }),
      );
    await open();
    await waitFor(() => expect(requests).toHaveLength(1));
    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(requests[0]!.signal?.aborted).toBe(true);
    await open();
    await waitFor(() => expect(requests).toHaveLength(2));
    await act(async () =>
      requests[0]!.resolve({
        sessions: [binding("same-session", "stale-provider-id")],
      }),
    );
    expect(screen.queryByText("stale-provider-id")).not.toBeInTheDocument();
    await act(async () =>
      requests[1]!.resolve({
        sessions: [binding("same-session", "fresh-provider-id")],
      }),
    );
    expect(await screen.findAllByText("fresh-provider-id")).toHaveLength(2);
    expect(screen.queryByText("stale-provider-id")).not.toBeInTheDocument();
  });

  it.each(["unavailable", "incomplete", "unsupported"] as const)(
    "labels %s history without offering an empty provider id",
    (history) => {
      render(
        <BindingTimeline
          result={{ type: "available", sessionId: "s", history, segments: [] }}
        />,
      );
      expect(
        screen.getByText(appI18n.t(`chat.bindingHistory_${history}`)),
      ).toBeInTheDocument();
      expect(
        screen.queryByText(appI18n.t("chat.bindingNotCreated")),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: appI18n.t("chat.bindingCopy") }),
      ).not.toBeInTheDocument();
    },
  );

  it.each([
    ["retry", "chat.bindingRetry"],
    ["not_found", "chat.bindingNotFound"],
    ["unavailable", "chat.bindingUnavailable"],
  ] as const)("distinguishes %s from never-created", (type, message) => {
    render(<BindingTimeline result={{ type, sessionId: "s" }} />);
    expect(screen.getByText(appI18n.t(message))).toBeInTheDocument();
    expect(
      screen.queryByText(appI18n.t("chat.bindingNotCreated")),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("shows a readable unbound session without an empty copy control", () => {
    render(
      <BindingTimeline
        result={{
          type: "available",
          sessionId: "s",
          history: "readable",
          segments: [],
        }}
      />,
    );
    expect(
      screen.getByText(appI18n.t("chat.bindingNotCreated")),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
  it("only queries on demand, preserves repeated segments and copies complete ids", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText");
    const read = vi.fn(({ sessionIds }: { sessionIds: string[] }) => ({
      sessions: sessionIds.map((id) => binding(id)),
    }));
    const Surface = fixture({ getSessionBindings: read });
    render(<Surface id="ora-1" />);
    expect(read).not.toHaveBeenCalled();
    expect(
      screen.queryByText("opaque/full-session-id"),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: appI18n.t("chat.sessionInformation"),
      }),
    );
    expect(await screen.findAllByText("opaque/full-session-id")).toHaveLength(
      2,
    );
    await user.click(
      screen.getAllByRole("button", {
        name: appI18n.t("chat.bindingCopy"),
      })[1]!,
    );
    expect(write).toHaveBeenCalledWith("opaque/full-session-id");
    expect(screen.getAllByTestId("run-diagnostics")).toHaveLength(1);
  });

  it("reserves one box for every copy feedback state so the row cannot reflow", async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined);
    const Surface = fixture({
      getSessionBindings: ({ sessionIds }) => ({
        sessions: sessionIds.map((id) => binding(id)),
      }),
    });
    render(<Surface id="ora-1" />);
    await user.click(
      screen.getByRole("button", {
        name: appI18n.t("chat.sessionInformation"),
      }),
    );
    await screen.findAllByText("opaque/full-session-id");
    const copy = screen.getAllByRole("button", {
      name: appI18n.t("chat.bindingCopy"),
    })[1]!;
    const row = copy.parentElement!;
    // Every label stays mounted in one grid cell, so the button is as wide as the longest one
    // before any click and the identifier beside it never shrinks while feedback is shown.
    for (const key of [
      "chat.bindingCopy",
      "chat.binding_copied",
      "chat.binding_copyFailed",
    ] as const) {
      expect(within(copy).getByText(appI18n.t(key))).toBeInTheDocument();
    }
    const status = within(row).getByRole("status");
    expect(status).toBeEmptyDOMElement();
    await user.click(copy);
    await waitFor(() =>
      expect(copy).toHaveAccessibleName(appI18n.t("chat.binding_copied")),
    );
    // Only the active label is exposed; the announcement comes from the persistent live region.
    expect(copy).not.toHaveAccessibleName(appI18n.t("chat.bindingCopy"));
    expect(status).toHaveTextContent(appI18n.t("chat.binding_copied"));
    expect(
      within(copy).getByText(appI18n.t("chat.bindingCopy")),
    ).toHaveAttribute("aria-hidden", "true");
  });

  it("reports a rejected clipboard write in the same button instead of a trailing label", async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(
      new Error("denied"),
    );
    const Surface = fixture({
      getSessionBindings: ({ sessionIds }) => ({
        sessions: sessionIds.map((id) => binding(id)),
      }),
    });
    render(<Surface id="ora-1" />);
    await user.click(
      screen.getByRole("button", {
        name: appI18n.t("chat.sessionInformation"),
      }),
    );
    await screen.findAllByText("opaque/full-session-id");
    const copy = screen.getAllByRole("button", {
      name: appI18n.t("chat.bindingCopy"),
    })[1]!;
    await user.click(copy);
    await waitFor(() =>
      expect(copy).toHaveAccessibleName(appI18n.t("chat.binding_copyFailed")),
    );
    expect(
      within(copy).getByText(appI18n.t("chat.binding_copyFailed")),
    ).toHaveAttribute("aria-hidden", "false");
    expect(within(copy.parentElement!).getByRole("status")).toHaveTextContent(
      appI18n.t("chat.binding_copyFailed"),
    );
  });

  it("aborts an old session query and never adopts its late response in the new panel", async () => {
    const user = userEvent.setup();
    let release!: (value: GetSessionBindingsResponse) => void;
    let oldSignal: AbortSignal | undefined;
    const Surface = fixture({
      getSessionBindings: ({ sessionIds }, options) => {
        if (sessionIds[0] === "old") {
          oldSignal = options?.signal;
          return new Promise((resolve) => {
            release = resolve;
          });
        }
        return { sessions: [binding("new", "new-provider-id")] };
      },
    });
    const view = render(<Surface id="old" />);
    await user.click(
      screen.getByRole("button", {
        name: appI18n.t("chat.sessionInformation"),
      }),
    );
    await waitFor(() => expect(oldSignal).toBeDefined());
    view.rerender(<Surface id="new" />);
    expect(oldSignal?.aborted).toBe(true);
    await user.click(
      screen.getByRole("button", {
        name: appI18n.t("chat.sessionInformation"),
      }),
    );
    await screen.findAllByText("new-provider-id");
    await act(async () => {
      release({ sessions: [binding("old", "old-provider-id")] });
    });
    expect(screen.queryByText("old-provider-id")).not.toBeInTheDocument();
  });

  it("does not mistake unavailable history for an uncreated provider", () => {
    render(
      <BindingTimeline
        result={{
          type: "available",
          sessionId: "s",
          history: "unavailable",
          segments: [],
        }}
      />,
    );
    expect(
      screen.getByText(appI18n.t("chat.bindingHistory_unavailable")),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(appI18n.t("chat.bindingNotCreated")),
    ).not.toBeInTheDocument();
  });
});
