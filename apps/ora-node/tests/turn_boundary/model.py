"""Loopback model responses that drive real Agent tools without credentials or model inference."""

import json
import shlex
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Model:
    def __init__(self, agent, scenario, workspace, output):
        self.agent = agent
        self.scenario = scenario
        self.workspace = workspace
        self.output = output
        self.requests = []
        self.task_issued = False
        self.tool_number = 0
        self.lock = threading.Lock()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), self.handler())
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server.server_port}"

    def handler(self):
        model = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                request = json.loads(
                    self.rfile.read(int(self.headers["Content-Length"]))
                )
                with model.lock:
                    model.requests.append(request)
                    (model.output / "model-requests.json").write_text(
                        json.dumps(model.requests, indent=2)
                    )
                if self.path.split("?")[0].endswith("/count_tokens"):
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    self.wfile.write(b'{"input_tokens":100}')
                    return
                tools = request.get("tools", [])
                if model.scenario == "error" and tools:
                    self.send_response(400)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    self.wfile.write(
                        b'{"type":"error","error":{"type":"invalid_request_error","message":"conformance model refusal"}}'
                    )
                    return
                messages = request.get("messages", [])
                has_result = any(
                    message.get("role") == "tool"
                    or any(
                        isinstance(block, dict) and block.get("type") == "tool_result"
                        for block in (
                            message.get("content")
                            if isinstance(message.get("content"), list)
                            else []
                        )
                    )
                    for message in messages
                )
                call = model.call(tools, has_result)
                # Keep the parent's prompt open after starting background work, so cancellation
                # occurs while that work is still alive, rather than after a normal turn end.
                if model.scenario == "background" and has_result and tools:
                    time.sleep(20)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                try:
                    if self.path.split("?")[0].endswith("/messages"):
                        self.anthropic(request, call)
                    else:
                        self.openai(request, call)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def event(self, name, data):
                prefix = f"event: {name}\n" if name else ""
                self.wfile.write(
                    (prefix + "data: " + json.dumps(data) + "\n\n").encode()
                )
                self.wfile.flush()

            def anthropic(self, request, call):
                self.event(
                    "message_start",
                    {
                        "type": "message_start",
                        "message": {
                            "id": "msg_probe",
                            "type": "message",
                            "role": "assistant",
                            "model": request["model"],
                            "content": [],
                            "stop_reason": None,
                            "stop_sequence": None,
                            "usage": {"input_tokens": 100, "output_tokens": 0},
                        },
                    },
                )
                if call:
                    name, arguments, identity = call
                    block = {
                        "type": "tool_use",
                        "id": identity,
                        "name": name,
                        "input": {},
                    }
                    delta = {
                        "type": "input_json_delta",
                        "partial_json": json.dumps(arguments),
                    }
                else:
                    block = {"type": "text", "text": ""}
                    delta = {"type": "text_delta", "text": "probe complete"}
                self.event(
                    "content_block_start",
                    {"type": "content_block_start", "index": 0, "content_block": block},
                )
                self.event(
                    "content_block_delta",
                    {"type": "content_block_delta", "index": 0, "delta": delta},
                )
                self.event(
                    "content_block_stop", {"type": "content_block_stop", "index": 0}
                )
                self.event(
                    "message_delta",
                    {
                        "type": "message_delta",
                        "delta": {
                            "stop_reason": "tool_use" if call else "end_turn",
                            "stop_sequence": None,
                        },
                        "usage": {"output_tokens": 10},
                    },
                )
                self.event("message_stop", {"type": "message_stop"})

            def openai(self, request, call):
                if call:
                    name, arguments, identity = call
                    delta = {
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": identity,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": json.dumps(arguments),
                                },
                            }
                        ],
                    }
                else:
                    delta = {"role": "assistant", "content": "probe complete"}
                for delta, finish in [
                    (delta, None),
                    ({}, "tool_calls" if call else "stop"),
                ]:
                    self.event(
                        None,
                        {
                            "id": "probe",
                            "object": "chat.completion.chunk",
                            "created": int(time.time()),
                            "model": request["model"],
                            "choices": [
                                {"index": 0, "delta": delta, "finish_reason": finish}
                            ],
                        },
                    )
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()

        return Handler

    def call(self, tools, has_result):
        if has_result or self.scenario not in ["shell", "subagent", "background"]:
            return None
        names = [
            tool.get("name", tool.get("function", {}).get("name", "")) for tool in tools
        ]
        shell = next((name for name in names if name.lower() == "bash"), None)
        task = next((name for name in names if name.lower() in ["task", "agent"]), None)
        with self.lock:
            self.tool_number += 1
            identity = f"probe-tool-{self.tool_number}"
            if self.scenario == "subagent" and task and not self.task_issued:
                self.task_issued = True
                return (
                    task,
                    {
                        "description": "cancel conformance child",
                        "prompt": "Execute the shell cancellation probe using Bash.",
                        "subagent_type": (
                            "general-purpose" if self.agent == "claude" else "general"
                        ),
                        "run_in_background": False,
                    },
                    identity,
                )
        if not shell:
            return None
        delay = 30 if self.scenario == "shell" else 3
        started = shlex.quote(str(self.workspace / "started"))
        leaked = shlex.quote(str(self.workspace / "leaked"))
        command = f"printf started > {started}; sleep {delay}; printf leaked > {leaked}"
        arguments = {
            "command": command,
            "description": "cancel conformance sleeper",
            "timeout": 60000,
        }
        if self.scenario == "background":
            if self.agent == "claude":
                arguments["run_in_background"] = True
            else:
                arguments["command"] = (
                    f"printf started > {started}; (sleep 3; printf leaked > {leaked}) >/dev/null 2>&1 &"
                )
        return shell, arguments, identity

    def close(self):
        self.server.shutdown()
        self.server.server_close()
