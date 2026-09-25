import { useEffect, useRef, useState } from "react";
import type {
  MiniCloneFailure,
  MiniCloneOperation,
  MiniCloneRequest,
} from "@ora/contracts";
import { Button } from "@ora/ui/components/button";
import { Input } from "@ora/ui/components/input";
import type { CloneClient } from "./client";
import { HttpError } from "./client";
import { readPending, writePending } from "./pending";

const reasons: Record<MiniCloneFailure, string> = {
  sourceUnavailable: "仓库暂不可用",
  branchNotFound: "找不到指定分支",
  destinationConflict: "目标目录冲突",
  operationFailed: "Git 操作失败",
  interrupted: "执行被中断，可重新提交",
};

/** Renders only Controller facts; pending is intentionally not a fabricated running/progress state. */
function Outcome({ operation }: { operation: MiniCloneOperation }) {
  const state = operation.state;
  switch (state.kind) {
    case "pending":
      return (
        <p className="text-muted-foreground">
          已接受 · 等待确定结果（可能离线或结果未知）
        </p>
      );
    case "succeeded":
      return (
        <div>
          <p className="text-emerald-700">Clone 完成</p>
          <p className="break-all font-mono text-xs">{state.commit}</p>
          <p className="break-all text-xs">Node 路径：{state.path}</p>
        </div>
      );
    case "failed":
      return (
        <div>
          <p className="text-destructive">{reasons[state.reason]}</p>
          {state.retainedPath && (
            <p className="break-all text-xs">
              残留已保留：{state.retainedPath}
            </p>
          )}
        </div>
      );
  }
}

/** Owns page-scoped requests and polling; reload restores unresolved submission identity, never auto-resubmits. */
export function App({
  client,
  storage = sessionStorage,
  pollMs = 1500,
}: {
  client: CloneClient;
  storage?: Storage;
  pollMs?: number;
}) {
  const [initial] = useState(() => {
    try {
      return { pending: readPending(storage), blocked: false };
    } catch {
      return { pending: null, blocked: true };
    }
  });
  const [pending, setPending] = useState<MiniCloneRequest | null>(
    initial.pending,
  );
  const [repository, setRepository] = useState(
    initial.pending?.repository ?? "",
  );
  const [branch, setBranch] = useState(initial.pending?.branch ?? "main");
  const [operations, setOperations] = useState<MiniCloneOperation[]>([]);
  const [queryError, setQueryError] = useState(false);
  const [message, setMessage] = useState(
    initial.blocked
      ? "无法读取本地待确认请求，请先检查浏览器存储；未发送新请求。"
      : "",
  );
  const [busy, setBusy] = useState(false);
  const [generation, refresh] = useState(0);
  const submitting = useRef<AbortController | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function poll() {
      try {
        const result = await client.list(controller.signal);
        if (!stopped) {
          setOperations(result);
          setQueryError(false);
        }
      } catch {
        if (!stopped) setQueryError(true);
      } finally {
        if (!stopped) timer = setTimeout(poll, pollMs);
      }
    }
    void poll();
    return () => {
      stopped = true;
      clearTimeout(timer);
      controller.abort();
    };
  }, [client, generation, pollMs]);

  useEffect(() => () => submitting.current?.abort(), []);

  async function submit(event: React.SubmitEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submitting.current || initial.blocked) return;
    const controller = new AbortController();
    submitting.current = controller;
    setBusy(true);
    setMessage("");
    try {
      const input = pending ?? {
        requestId: crypto.randomUUID(),
        repository,
        branch,
      };
      writePending(storage, input);
      setPending(input);
      const accepted = await client.submit(input, controller.signal);
      if (controller.signal.aborted) return;
      writePending(storage, null);
      setPending(null);
      setMessage(`已持久接受：${accepted.executionId}`);
      refresh((value) => value + 1);
    } catch (error) {
      if (!controller.signal.aborted) {
        setMessage(
          error instanceof HttpError && error.code === "invalidInput"
            ? "仓库或分支格式无效。"
            : error instanceof HttpError && error.code === "conflict"
              ? "请求身份与原输入冲突，未创建新执行。"
              : "尚未确认接受结果。可重试原请求，不会主动创建新的执行身份。",
        );
        if (
          error instanceof HttpError &&
          (error.code === "invalidInput" || error.code === "conflict")
        ) {
          try {
            writePending(storage, null);
            setPending(null);
          } catch {
            // Keep the original identity locked until browser storage can forget it too.
            setMessage(
              "无法清除本地待确认请求；请恢复浏览器存储后重试原请求。",
            );
          }
        }
      }
    } finally {
      submitting.current = null;
      if (!controller.signal.aborted) setBusy(false);
    }
  }

  return (
    <main className="mx-auto max-w-5xl px-6 py-12">
      <header className="mb-10 flex items-center justify-between border-b pb-6">
        <div>
          <p className="text-xs font-semibold tracking-widest text-muted-foreground">
            ORA / LOCAL DEVELOPMENT
          </p>
          <h1 className="mt-2 text-3xl font-semibold tracking-tight">
            minicloud
          </h1>
        </div>
        <span className="rounded-full border px-3 py-1 text-xs">
          非生产 · 本机
        </span>
      </header>
      <div className="grid items-start gap-8 md:grid-cols-[320px_1fr]">
        <section className="rounded-xl border bg-card p-6 shadow-sm">
          <h2 className="mb-2 text-lg font-semibold">获取仓库</h2>
          <p className="mb-6 text-sm text-muted-foreground">
            由 Node clone 指定分支，保留完整历史。
          </p>
          <form onSubmit={(event) => void submit(event)} className="space-y-4">
            <label className="block space-y-2 text-sm">
              仓库 URL
              <Input
                required
                value={repository}
                disabled={busy || !!pending || initial.blocked}
                onChange={(event) => setRepository(event.target.value)}
                placeholder="https://… / ssh://…"
              />
            </label>
            <label className="block space-y-2 text-sm">
              分支
              <Input
                required
                value={branch}
                disabled={busy || !!pending || initial.blocked}
                onChange={(event) => setBranch(event.target.value)}
              />
            </label>
            <Button
              type="submit"
              className="w-full"
              disabled={busy || initial.blocked}
            >
              {busy ? "正在提交…" : pending ? "重试原请求" : "提交 clone"}
            </Button>
          </form>
          {message && (
            <p role="status" className="mt-4 break-all text-sm">
              {message}
            </p>
          )}
          <p className="mt-6 text-xs text-muted-foreground">
            关闭页面不会取消已接受的执行。失败残留不会自动删除。
          </p>
        </section>
        <section>
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-lg font-semibold">执行记录</h2>
            <span className="text-xs text-muted-foreground">自动查询</span>
          </div>
          {queryError && (
            <p role="alert" className="mb-4 text-sm text-amber-700">
              暂时无法查询，已有执行状态未改变。
            </p>
          )}
          {!operations.length && (
            <p className="rounded-xl border border-dashed p-8 text-sm text-muted-foreground">
              {queryError ? "等待恢复连接" : "暂无已接受的操作"}
            </p>
          )}
          <ul className="space-y-4">
            {operations.map((operation) => (
              <li
                key={operation.executionId}
                className="rounded-xl border bg-card p-5 text-sm"
              >
                <h3 className="break-all font-medium">
                  {operation.repository}
                </h3>
                <p className="mb-4 mt-1 text-xs text-muted-foreground">
                  {operation.branch} · {operation.nodeId}
                </p>
                <Outcome operation={operation} />
                <p className="mt-4 break-all font-mono text-[10px] text-muted-foreground">
                  {operation.executionId}
                </p>
              </li>
            ))}
          </ul>
        </section>
      </div>
    </main>
  );
}
