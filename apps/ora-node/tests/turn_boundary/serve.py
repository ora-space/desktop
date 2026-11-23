#!/usr/bin/env python3
"""Serve the probe's loopback model to a real Node run until stdin closes.

The Node's opt-in turn boundary conformance test starts this, so a run through the Node drives
the Agent exactly as `probe.py` does. It prints one JSON line with the model URL and the Agent
environment `probe.py` uses, then serves `--scenario` while `<output>/plain` is absent and plain
answers while it exists, so the test chooses which turn exercises the scenario. Tool commands use
paths relative to the shell's working directory: the markers land in the session checkout.
"""

import argparse
import json
from pathlib import Path
import sys
import threading
import time

sys.dont_write_bytecode = True

from model import Model
from probe import environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", choices=["opencode", "claude"], required=True)
    parser.add_argument(
        "--scenario",
        choices=["shell", "subagent", "background", "detached", "error"],
        required=True,
    )
    parser.add_argument("--home", type=Path, required=True, help="the Agent's new HOME")
    parser.add_argument("--output", type=Path, required=True, help="an existing directory")
    args = parser.parse_args()
    model = Model(args.agent, args.scenario, Path("."), args.output)
    plain = args.output / "plain"

    def follow():
        while True:
            model.scenario = "plain" if plain.exists() else args.scenario
            time.sleep(0.01)

    threading.Thread(target=follow, daemon=True).start()
    print(
        json.dumps(
            {"url": model.url, "environment": environment(args.agent, args.home, model)}
        ),
        flush=True,
    )
    sys.stdin.read()
    (args.output / "wakes.json").write_text(json.dumps(model.wakes))
    model.close()


if __name__ == "__main__":
    main()
