#!/usr/bin/env python3
"""Probe a real installed Agent's cancellation fence through its unmodified Ora plugin.

The loopback model drives real tools deterministically. No external model or login is used.
This is an opt-in Linux conformance probe, not a statistical estimate of workload coverage.

Besides the traffic and file writes it observes after the answer, the probe lists the processes
the turn started that are still alive when the Agent answers. The host is a child subreaper, so
that list includes jobs that left the Agent's process group or session. A file write only shows
work that finished inside the observation window; a live process shows unfinished work whatever
it is waiting for.
"""

import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time

sys.dont_write_bytecode = True

from model import Model
from plugin_host import PluginHost
from processes import descendants, started_since

SESSION_UPDATES = {
    "session_info_update",
    "available_commands_update",
    "current_mode_update",
    "config_option_update",
    "usage_update",
}


def environment(agent, home, model):
    """Keep host credentials and user configuration out of the real Agent under test."""
    env = {
        key: value for key, value in os.environ.items() if key in {"PATH", "LANG", "TZ"}
    }
    env.update(
        HOME=str(home),
        XDG_CONFIG_HOME=str(home / "config"),
        XDG_DATA_HOME=str(home / "data"),
        XDG_CACHE_HOME=str(home / "cache"),
        CLAUDE_CONFIG_DIR=str(home / "claude"),
        DENO_DIR=str(home / "deno"),
    )
    if agent == "opencode":
        config = {
            "model": "probe/probe-model",
            "small_model": "probe/probe-model",
            "enabled_providers": ["probe"],
            "provider": {
                "probe": {
                    "npm": "@ai-sdk/openai-compatible",
                    "options": {"baseURL": model.url + "/v1", "apiKey": "probe"},
                    "models": {
                        "probe-model": {
                            "name": "probe",
                            "limit": {"context": 128000, "output": 4096},
                        }
                    },
                }
            },
            "permission": "allow",
        }
        env.update(
            OPENCODE_CONFIG_CONTENT=json.dumps(config),
            OPENCODE_CONFIG_DIR=str(home / "config"),
            OPENCODE_DISABLE_PROJECT_CONFIG="true",
            OPENCODE_CONFIG_PROJECT_DISABLE="true",
            OPENCODE_DISABLE_AUTOUPDATE="true",
            OPENCODE_DISABLE_MODELS_FETCH="true",
        )
    else:
        env.update(
            ANTHROPIC_BASE_URL=model.url,
            ANTHROPIC_API_KEY="probe",
            ANTHROPIC_AUTH_TOKEN="probe",
            API_MAX_RETRIES="0",
            ANTHROPIC_MODEL="claude-sonnet-4-5",
            ANTHROPIC_SMALL_FAST_MODEL="claude-sonnet-4-5",
            CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1",
            DISABLE_AUTOUPDATER="1",
            ORA_CLAUDE_BUNDLED_CLI_ONLY="1",
        )
    return env


def turn_traffic(entry):
    """Apply the ADR's distinction between turn traffic and legitimate idle notifications."""
    frame = entry["frame"]
    return frame.get("method") == "session/request_permission" or (
        frame.get("method") == "session/update"
        and frame.get("params", {}).get("update", {}).get("sessionUpdate")
        not in SESSION_UPDATES
    )


def package_identity(package):
    """Record exact package metadata and binary digests alongside every observation."""
    manifest = (package / "orax.toml").read_text()
    fields = {
        key: re.search(r"^" + key + r'\s*=\s*"([^\"]+)"', manifest, re.M).group(1)
        for key in ["identifier", "version"]
    }
    files = [package / "main.js", *sorted((package / "assets" / "bin").iterdir())]
    fields["files_sha256"] = {
        str(path.relative_to(package)): sha256(path) for path in files if path.is_file()
    }
    return fields


def sha256(path):
    """Hash large packaged executables without retaining their bytes in memory."""
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


async def probe(args):
    output = args.output.resolve()
    output.mkdir(parents=True)
    home, workspace = output / "home", output / "work"
    home.mkdir()
    workspace.mkdir()
    model = Model(args.agent, args.scenario, workspace, output)
    events, pending = [], {}
    summary = {
        "package": package_identity(args.package.resolve()),
        "scenario": args.scenario,
        "observation_seconds": args.observe_seconds,
    }
    with (output / "stderr.log").open("w") as stderr:
        host = PluginHost(
            args.package.resolve(),
            workspace,
            environment(args.agent, home, model),
            stderr,
        )
        reader = None

        async def read():
            async for frame in host.frames():
                at = time.monotonic()
                events.append({"at": at, "frame": frame})
                (output / "acp.json").write_text(json.dumps(events, indent=2))
                if "id" in frame and "method" not in frame:
                    future = pending.pop(frame["id"], None)
                    if future and not future.done():
                        future.set_result((at, frame))
                elif frame.get("method") == "session/request_permission":
                    allowed = next(
                        option
                        for option in frame["params"]["options"]
                        if option["kind"].startswith("allow")
                    )
                    await host.send(
                        {
                            "jsonrpc": "2.0",
                            "id": frame["id"],
                            "result": {
                                "outcome": {
                                    "outcome": "selected",
                                    "optionId": allowed["optionId"],
                                }
                            },
                        }
                    )
                elif "id" in frame:
                    await host.send(
                        {
                            "jsonrpc": "2.0",
                            "id": frame["id"],
                            "error": {
                                "code": -32601,
                                "message": "unsupported client request",
                            },
                        }
                    )

        async def rpc(identity, method, params):
            future = asyncio.get_running_loop().create_future()
            pending[identity] = future
            await host.send(
                {"jsonrpc": "2.0", "id": identity, "method": method, "params": params}
            )
            return await asyncio.wait_for(future, 60)

        try:
            await host.start()
            reader = asyncio.create_task(read())
            _, initialized = await rpc(
                1,
                "initialize",
                {
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": {"name": "ora-turn-boundary-probe", "version": "1"},
                },
            )
            summary["agent_info"] = initialized["result"].get("agentInfo")
            _, created = await rpc(
                2, "session/new", {"cwd": str(workspace), "mcpServers": []}
            )
            session = created["result"]["sessionId"]
            before_prompt = descendants()
            prompt = asyncio.create_task(
                rpc(
                    3,
                    "session/prompt",
                    {
                        "sessionId": session,
                        "prompt": [
                            {
                                "type": "text",
                                "text": "Run the cancellation conformance task using the available tools.",
                            }
                        ],
                    },
                )
            )
            cancelled_at = None
            if args.scenario != "error":
                for _ in range(600):
                    if (workspace / "started").exists() or prompt.done():
                        break
                    await asyncio.sleep(0.1)
                if not (workspace / "started").exists() or prompt.done():
                    raise RuntimeError(
                        "the Agent did not run the target tool with its prompt still open"
                    )
                await asyncio.sleep(0.25)
                if prompt.done():
                    raise RuntimeError(
                        "the target prompt completed before cancellation"
                    )
                cancelled_at = time.monotonic()
                await host.send(
                    {
                        "jsonrpc": "2.0",
                        "method": "session/cancel",
                        "params": {"sessionId": session},
                    }
                )
            answered_at, answer = await prompt
            turn_processes = started_since(before_prompt)
            summary["response"] = answer
            summary["turn_processes_at_response"] = turn_processes
            written_at_response = (workspace / "leaked").exists()
            woke_at_response = (workspace / "woke").exists()
            summary["background_write_at_response"] = written_at_response
            tools = {}
            for entry in events:
                if entry["at"] > answered_at:
                    continue
                update = entry["frame"].get("params", {}).get("update", {})
                if update.get("toolCallId") and update.get("status"):
                    tools[update["toolCallId"]] = update["status"]
            summary["tool_status_at_response"] = tools
            summary["cancel_ms"] = (
                round(1000 * (answered_at - cancelled_at), 3) if cancelled_at else None
            )
            await asyncio.sleep(args.observe_seconds)
            late = [
                entry
                for entry in events
                if entry["at"] > answered_at and turn_traffic(entry)
            ]
            summary["late_turn_traffic"] = late
            summary["background_write_after_response"] = (
                not written_at_response and (workspace / "leaked").exists()
            )
            # Requests the Agent started by itself about its background work, and whether the
            # shell call the fixture answered them with ran after the answer.
            summary["wake_requests_after_response"] = [
                round(at - answered_at, 3) for at in model.wakes if at > answered_at
            ]
            summary["wake_write_after_response"] = (
                not woke_at_response and (workspace / "woke").exists()
            )
            summary["model_requests"] = len(model.requests)
            summary["tool_names"] = [
                entry["frame"].get("params", {}).get("update", {}).get("title")
                for entry in events
                if entry["frame"]
                .get("params", {})
                .get("update", {})
                .get("sessionUpdate")
                == "tool_call"
            ]
            # A model error is not proof the provider id was discarded. Ask that id again and
            # report the actual response instead of assuming an Agent-specific recovery rule.
            model.scenario = "plain"
            _, continued = await rpc(
                4,
                "session/prompt",
                {
                    "sessionId": session,
                    "prompt": [
                        {"type": "text", "text": "Continue with a short answer."}
                    ],
                },
            )
            summary["next_prompt"] = continued
            summary["subagent_started"] = model.task_issued
            target_answered = (
                "error" in answer
                if args.scenario == "error"
                else answer.get("result", {}).get("stopReason") == "cancelled"
            )
            summary["conformant_in_observation"] = (
                target_answered
                and (args.scenario != "subagent" or model.task_issued)
                and not late
                and not turn_processes
                and not written_at_response
                and not summary["background_write_after_response"]
                and not summary["wake_write_after_response"]
                and continued.get("result", {}).get("stopReason") == "end_turn"
            )
        except Exception as error:
            summary["probe_error"] = str(error) or type(error).__name__
            summary["conformant_in_observation"] = False
        finally:
            await host.close()
            if reader:
                reader.cancel()
                await asyncio.gather(reader, return_exceptions=True)
            model.close()
            (output / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))
    return 0 if summary["conformant_in_observation"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", choices=["opencode", "claude"], required=True)
    parser.add_argument(
        "--package",
        type=Path,
        required=True,
        help="extracted, unmodified .orax package",
    )
    parser.add_argument(
        "--scenario",
        choices=["shell", "subagent", "background", "detached", "error"],
        required=True,
    )
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="new directory outside the repository",
    )
    parser.add_argument("--observe-seconds", type=float, default=5)
    args = parser.parse_args()
    if args.observe_seconds < 3:
        parser.error("observe-seconds must cover the three-second background write")
    raise SystemExit(asyncio.run(probe(args)))


if __name__ == "__main__":
    main()
