"""Minimal Ora framed host for exercising the unmodified installed agent package."""

import asyncio
import base64
import json
import os
import signal
import struct

from processes import become_subreaper, kill_orphans


class PluginHost:
    def __init__(self, package, cwd, env, stderr):
        self.package = package
        self.cwd = cwd
        self.env = env
        self.stderr = stderr
        self.children = {}
        self.inbound = asyncio.Queue()
        self.pending = {}
        self.registered = asyncio.Event()
        self.tasks = []
        self.lock = asyncio.Lock()
        self.plugin = None

    async def start(self):
        become_subreaper()
        self.plugin = await asyncio.create_subprocess_exec(
            "deno",
            "run",
            "--no-prompt",
            "--allow-run",
            "--allow-read",
            "--allow-env",
            "--allow-net",
            str(self.package / "main.js"),
            cwd=self.package,
            env=self.env,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=self.stderr,
            start_new_session=True,
        )
        self.tasks.append(asyncio.create_task(self.read()))
        await asyncio.wait_for(self.registered.wait(), 20)
        self.pending["start"] = asyncio.get_running_loop().create_future()
        await self.write(
            {
                "jsonrpc": "2.0",
                "id": "start",
                "method": "agent/start",
                "params": {"cwd": str(self.cwd), "hostVersion": "0.8.0"},
            }
        )
        answer = await asyncio.wait_for(self.pending["start"], 60)
        if "error" in answer:
            raise RuntimeError(answer["error"])

    async def write(self, frame):
        data = json.dumps(frame).encode()
        async with self.lock:
            self.plugin.stdin.write(struct.pack(">I", len(data) + 1) + b"\x01" + data)
            await self.plugin.stdin.drain()

    async def send(self, frame):
        await self.write({"jsonrpc": "2.0", "method": "agent/acp", "params": frame})

    async def frames(self):
        while True:
            frame = await self.inbound.get()
            if frame is None:
                return
            yield frame

    async def read(self):
        try:
            while True:
                length = struct.unpack(">I", await self.plugin.stdout.readexactly(4))[0]
                data = await self.plugin.stdout.readexactly(length)
                if data[0] != 1:
                    raise RuntimeError("unsupported plugin frame")
                frame = json.loads(data[1:])
                if frame.get("method") == "ora/register":
                    self.registered.set()
                elif frame.get("method") == "agent/acp":
                    await self.inbound.put(frame["params"])
                elif (
                    str(frame.get("method", "")).startswith("ora/childprocess/")
                    and "id" in frame
                ):
                    self.tasks.append(asyncio.create_task(self.dispatch(frame)))
                elif frame.get("id") in self.pending:
                    self.pending[frame["id"]].set_result(frame)
        except asyncio.IncompleteReadError:
            await self.inbound.put(None)

    async def dispatch(self, frame):
        params = frame.get("params", {})
        method = frame["method"].rsplit("/", 1)[-1]
        try:
            if method == "spawn":
                command = (
                    str(self.package / params["packageCommand"])
                    if "packageCommand" in params
                    else params["command"]
                )
                child = await asyncio.create_subprocess_exec(
                    command,
                    *params.get("args", []),
                    cwd=params.get("cwd") or self.cwd,
                    env={**self.env, **params.get("env", {})},
                    stdin=asyncio.subprocess.PIPE,
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE,
                    start_new_session=True,
                )
                process_id = str(len(self.children) + 1)
                self.children[process_id] = child
                # Publish the spawn result before any output from that process.
                await self.write(
                    {
                        "jsonrpc": "2.0",
                        "id": frame["id"],
                        "result": {"processId": process_id, "pid": child.pid},
                    }
                )
                self.tasks.extend(
                    [
                        asyncio.create_task(
                            self.pump(process_id, child.stdout, "stdout")
                        ),
                        asyncio.create_task(
                            self.pump(process_id, child.stderr, "stderr")
                        ),
                        asyncio.create_task(self.exited(process_id, child)),
                    ]
                )
                return
            child = self.children[params["processId"]]
            if method == "write":
                child.stdin.write(base64.b64decode(params["bytesBase64"]))
                await child.stdin.drain()
            elif method == "close_stdin":
                child.stdin.close()
            elif method == "kill":
                child.kill()
            else:
                raise RuntimeError("unsupported host method " + method)
            await self.write({"jsonrpc": "2.0", "id": frame["id"], "result": {}})
        except Exception as error:
            await self.write(
                {
                    "jsonrpc": "2.0",
                    "id": frame["id"],
                    "error": {
                        "code": -32000,
                        "message": str(error),
                        "data": {"kind": "io"},
                    },
                }
            )

    async def pump(self, process_id, stream, name):
        while data := await stream.read(65536):
            await self.write(
                {
                    "jsonrpc": "2.0",
                    "method": "ora/childprocess/" + name,
                    "params": {
                        "processId": process_id,
                        "bytesBase64": base64.b64encode(data).decode(),
                    },
                }
            )

    async def exited(self, process_id, child):
        code = await child.wait()
        await self.write(
            {
                "jsonrpc": "2.0",
                "method": "ora/childprocess/exit",
                "params": {"processId": process_id, "code": code, "signal": None},
            }
        )

    async def close(self):
        children = [*self.children.values(), *([self.plugin] if self.plugin else [])]
        for child in children:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for child in children:
            await child.wait()
        # Agents start tool shells in their own sessions, so killing the process groups above
        # leaves their jobs running; the subreaper still owns them.
        kill_orphans({child.pid for child in children})
        for task in self.tasks:
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
