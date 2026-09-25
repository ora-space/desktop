import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MiniCloneOperation, MiniCloneRequest } from "@ora/contracts";
import { App } from "./App";
import type { CloneClient } from "./client";
import { HttpError } from "./client";

const operation: MiniCloneOperation = {
  operationId: "operation",
  executionId: "execution",
  nodeId: "node",
  repository: "https://example.com/repo.git",
  branch: "main",
  state: { kind: "pending" },
};

beforeEach(() => sessionStorage.clear());

describe("clone application", () => {
  it("automatically resumes polling after an outage without submitting again", async () => {
    let online = false;
    const submit = vi.fn<CloneClient["submit"]>();
    render(
      <App
        pollMs={20}
        client={{
          submit,
          list: async () => {
            if (!online) throw new Error("offline");
            return [
              {
                ...operation,
                state: {
                  kind: "succeeded",
                  path: "/restored",
                  commit: "original-commit",
                },
              },
            ];
          },
        }}
      />,
    );
    await screen.findByRole("alert");
    online = true;
    await screen.findByText("original-commit");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText("Node 路径：/restored")).toBeTruthy();
    expect(submit).not.toHaveBeenCalled();
  });
  it("keeps the request locked when rejected input cannot be removed from storage", async () => {
    const storage: Storage = {
      length: 0,
      clear: vi.fn(),
      key: () => null,
      getItem: () => null,
      setItem: vi.fn(),
      removeItem: () => {
        throw new Error("storage unavailable");
      },
    };
    const user = userEvent.setup();
    render(
      <App
        storage={storage}
        client={{
          list: async () => [],
          submit: async () => {
            throw new HttpError("invalidInput");
          },
        }}
      />,
    );
    await user.type(screen.getByLabelText("仓库 URL"), operation.repository);
    await user.click(screen.getByRole("button", { name: "提交 clone" }));
    await screen.findByText(/无法清除本地待确认请求/);
    expect(
      (screen.getByLabelText("仓库 URL") as HTMLInputElement).disabled,
    ).toBe(true);
    expect(screen.getByRole("button", { name: "重试原请求" })).toBeTruthy();
  });
  it("shows known failure and preserved residue without resubmitting", async () => {
    const submit = vi.fn<CloneClient["submit"]>();
    const failed: MiniCloneOperation = {
      ...operation,
      state: {
        kind: "failed",
        reason: "branchNotFound",
        retainedPath: "/node/retained",
      },
    };
    render(<App client={{ list: async () => [failed], submit }} />);
    await screen.findByText("找不到指定分支");
    expect(screen.getByText("残留已保留：/node/retained")).toBeTruthy();
    expect(submit).not.toHaveBeenCalled();
  });
  it("tells an interrupted attempt apart from a Git failure", async () => {
    const interrupted: MiniCloneOperation = {
      ...operation,
      state: {
        kind: "failed",
        reason: "interrupted",
        retainedPath: "/node/cut",
      },
    };
    render(
      <App client={{ list: async () => [interrupted], submit: vi.fn() }} />,
    );
    await screen.findByText("执行被中断，可重新提交");
    expect(screen.getByText("残留已保留：/node/cut")).toBeTruthy();
  });
  it("retains the same submission across response loss and reload", async () => {
    const requests: MiniCloneRequest[] = [];
    const client: CloneClient = {
      list: async () => (requests.length ? [operation] : []),
      submit: async (input) => {
        requests.push(input);
        if (requests.length === 1) throw new Error("response lost");
        return {
          requestId: input.requestId,
          operationId: "operation",
          executionId: "execution",
        };
      },
    };
    const user = userEvent.setup();
    const first = render(<App client={client} />);
    await user.type(screen.getByLabelText("仓库 URL"), operation.repository);
    await user.click(screen.getByRole("button", { name: "提交 clone" }));
    await screen.findByText(/尚未确认接受结果/);
    first.unmount();
    render(<App client={client} />);
    await screen.findByText(/等待确定结果/);
    expect(
      (screen.getByLabelText("仓库 URL") as HTMLInputElement).disabled,
    ).toBe(true);
    await user.click(screen.getByRole("button", { name: "重试原请求" }));
    await screen.findByText("已持久接受：execution");
    expect(requests).toEqual([requests[0], requests[0]]);
    expect(sessionStorage.length).toBe(0);
  });

  it("keeps terminal facts visible when polling fails", async () => {
    const succeeded: MiniCloneOperation = {
      ...operation,
      state: { kind: "succeeded", path: "/node/repository", commit: "abc123" },
    };
    const list = vi
      .fn<CloneClient["list"]>()
      .mockResolvedValueOnce([succeeded])
      .mockRejectedValue(new Error("offline"));
    const client: CloneClient = {
      list,
      submit: async () => {
        throw new Error("unexpected submit");
      },
    };
    const page = render(<App client={client} pollMs={20} />);
    await screen.findByText("Clone 完成");
    await screen.findByRole("alert");
    expect(screen.getByText("abc123")).toBeTruthy();
    expect(screen.getByText("Node 路径：/node/repository")).toBeTruthy();
    page.unmount();
    expect(list.mock.calls.every(([signal]) => signal.aborted)).toBe(true);
  });

  it("aborts an in-flight poll when the page is unmounted", async () => {
    let signal: AbortSignal | undefined;
    const client: CloneClient = {
      list: (input) => {
        signal = input;
        return new Promise((_, reject) =>
          input.addEventListener("abort", () => reject(new Error("aborted")), {
            once: true,
          }),
        );
      },
      submit: async () => {
        throw new Error("unexpected submit");
      },
    };
    const page = render(<App client={client} />);
    await waitFor(() => expect(signal).toBeDefined());
    page.unmount();
    expect(signal?.aborted).toBe(true);
  });
});
