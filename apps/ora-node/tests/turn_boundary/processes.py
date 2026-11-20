"""Linux process accounting for the probe host: which processes a turn left running.

The probe host is made a child subreaper, so a process the Agent tree orphans (for example a
shell job started with `&` whose parent shell already exited, or a job that called `setsid`)
reparents to the host instead of init. Every process the Agent started therefore stays a host
descendant, whatever process group or session it moved to, and can be listed and killed.
"""

import ctypes
import os
import signal
import time

PR_SET_CHILD_SUBREAPER = 36


def become_subreaper():
    """Keep orphans of the Agent tree under this process; the probe is Linux-only."""
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "prctl(PR_SET_CHILD_SUBREAPER)")


def read(pid):
    """One process's identity and grouping, or None once it is gone."""
    try:
        with open(f"/proc/{pid}/stat") as source:
            raw = source.read()
        with open(f"/proc/{pid}/cmdline", "rb") as source:
            command = source.read().replace(b"\0", b" ").decode(errors="replace")
    except OSError:
        return None
    # The command name may contain spaces or parentheses, so split after its closing one.
    fields = raw[raw.rindex(")") + 2 :].split()
    return {
        "pid": pid,
        "ppid": int(fields[1]),
        "pgid": int(fields[2]),
        "sid": int(fields[3]),
        "state": fields[0],
        "start_ticks": int(fields[19]),
        "command": command.strip()[:200],
    }


def descendants():
    """Every live process below this one, keyed by (pid, start) so pid reuse cannot alias."""
    processes = [
        process
        for process in map(read, (int(name) for name in os.listdir("/proc") if name.isdigit()))
        if process
    ]
    found, parents = {}, {os.getpid()}
    while True:
        children = [
            process
            for process in processes
            if process["ppid"] in parents and process["pid"] not in parents
        ]
        if not children:
            return found
        for process in children:
            found[(process["pid"], process["start_ticks"])] = process
        parents.update(process["pid"] for process in children)


def started_since(before):
    """Processes that did not exist in the `before` snapshot and have not exited yet."""
    sessions = {process["sid"] for process in before.values()}
    return [
        {**process, "agent_session": process["sid"] in sessions}
        for key, process in descendants().items()
        if key not in before and process["state"] != "Z"
    ]


def kill_orphans(managed):
    """Kill every remaining descendant and reap the orphans that reparented to this process.

    `managed` holds the pids asyncio spawned; their exit status belongs to asyncio, so they are
    left for it to reap.
    """
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        remaining = descendants().values()
        if not remaining:
            return
        for process in remaining:
            try:
                os.kill(process["pid"], signal.SIGKILL)
            except ProcessLookupError:
                pass
            if process["ppid"] == os.getpid() and process["pid"] not in managed:
                try:
                    os.waitpid(process["pid"], os.WNOHANG)
                except ChildProcessError:
                    pass
        time.sleep(0.01)
    raise RuntimeError("descendants of the probe host survived SIGKILL")
