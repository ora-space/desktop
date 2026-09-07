import assert from "node:assert/strict";

const wrapper = new URL("./run-with-clean-stderr.ts", import.meta.url);

// Exercise the process boundary: checking only the wrapper's bookkeeping would
// miss task-shell diagnostics, lost exit codes, and broken command resolution.
for (const scenario of [
  {
    name: "accepts clean stdout",
    command: "echo clean",
    code: 0,
    stdout: "clean",
    stderr: "",
  },
  {
    name: "rejects stderr despite success",
    command: "echo warning >&2",
    code: 1,
    stdout: "",
    stderr: "test command wrote to stderr",
  },
  {
    name: "preserves a failing command's exit code",
    command: "exit 7",
    code: 7,
    stdout: "",
    stderr: "",
  },
  {
    name: "resolves npm tools without Node",
    command: "tsc --version",
    code: 0,
    stdout: "Version",
    stderr: "",
  },
]) {
  Deno.test(scenario.name, async () => {
    const output = await new Deno.Command(Deno.execPath(), {
      args: ["run", "-A", wrapper.href, scenario.command],
      cwd: new URL("../", import.meta.url),
      stdout: "piped",
      stderr: "piped",
    }).output();
    const stdout = new TextDecoder().decode(output.stdout).trim();
    const stderr = new TextDecoder().decode(output.stderr).trim();
    assert.equal(output.code, scenario.code, stderr);
    assert.ok(stdout.startsWith(scenario.stdout), stdout);
    if (scenario.stderr === "") assert.equal(stderr, "");
    else assert.ok(stderr.includes(scenario.stderr), stderr);
  });
}
