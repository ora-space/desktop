import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import path from "node:path";
import { setupBinaries, type SetupOptions } from "./setup-binary.ts";
import { withTempDirectory } from "./test-support.ts";

/** Supplies reviewed fixture hashes while exercising real staging, hashing, and publication. */
async function fixture(
  root: string,
  name: "deno" | "rg" = "deno",
  target = "x86_64-unknown-linux-gnu",
) {
  const windows = target.includes("windows");
  const executable = name + (windows ? ".exe" : "");
  const version = name === "deno" ? "2.9.5" : "15.2.0";
  const prefix = name === "deno" ? "deno" : "ripgrep";
  const extension = name === "deno" || windows ? "zip" : "tar.gz";
  const assetTarget =
    name === "rg" && target.endsWith("linux-gnu")
      ? "x86_64-unknown-linux-musl"
      : target;
  const asset = `${prefix}-${name === "deno" ? "" : `${version}-`}${assetTarget}.${extension}`;
  const url = `https://github.com/${name === "deno" ? "denoland/deno" : "BurntSushi/ripgrep"}/releases/download/${name === "deno" ? "v" : ""}${version}/${asset}`;
  const directory = path.join(root, "apps", "desktop", "src-tauri", "binaries");
  const destination = path.join(
    directory,
    `${name}-${target}${windows ? ".exe" : ""}`,
  );
  const archive = "trusted fixture archive";
  const binary = "trusted fixture executable";
  await Deno.mkdir(path.join(root, "scripts"));
  await Deno.writeTextFile(path.join(root, ".deno-version"), "2.9.5\n");
  await Deno.writeTextFile(
    path.join(root, "scripts", "sidecar-checksums.json"),
    JSON.stringify({
      [url]: {
        archiveSha256: createHash("sha256").update(archive).digest("hex"),
        executableSha256: createHash("sha256").update(binary).digest("hex"),
      },
    }),
  );
  const commands: { command: string; args: string[] }[] = [];
  const exec: NonNullable<SetupOptions["exec"]> = async (command, args) => {
    commands.push({ command, args });
    if (command === "curl") {
      assert.equal(args.at(-1), url);
      await Deno.writeTextFile(args[args.indexOf("--output") + 1], archive);
    } else if (
      command === "tar" ||
      command === "unzip" ||
      command === "powershell"
    ) {
      const extracted =
        command === "powershell"
          ? args.at(-1)!.match(/-DestinationPath '([^']+)'/)![1]
          : args[args.indexOf(command === "tar" ? "-C" : "-d") + 1];
      await Deno.mkdir(path.join(extracted, "nested"));
      await Deno.writeTextFile(
        path.join(extracted, "nested", executable),
        binary,
      );
    } else {
      assert.equal(
        path.basename(command) === executable || command === destination,
        true,
      );
      assert.deepEqual(args, ["--version"]);
      return { stdout: `${prefix} ${version} (stable, release)\r\n` };
    }
  };
  const options: SetupOptions = {
    root,
    args: [name],
    environment: { TARGET_TRIPLE: target },
    platform: windows
      ? "win32"
      : target.includes("darwin")
        ? "darwin"
        : "linux",
    arch: target.startsWith("aarch64") ? "arm64" : "x64",
    exec,
  };
  return {
    options,
    commands,
    directory,
    destination,
    archive,
    binary,
    url,
    exec,
  };
}

for (const [name, target] of [
  ["deno", "x86_64-unknown-linux-gnu"],
  ["rg", "x86_64-unknown-linux-gnu"],
  ["deno", "x86_64-pc-windows-msvc"],
  ["rg", "x86_64-pc-windows-msvc"],
  ["deno", "aarch64-apple-darwin"],
] as const) {
  Deno.test(
    `${name} ${target} verifies downloads and reuses only verified cache`,
    () =>
      withTempDirectory(async (root) => {
        const f = await fixture(root, name, target);
        f.options.environment!.HTTPS_PROXY = "http://fixture-proxy";
        await setupBinaries(f.options);
        assert.equal(await Deno.readTextFile(f.destination), f.binary);
        assert.deepEqual(f.commands[0].args.slice(-3, -1), [
          "--proxy",
          "http://fixture-proxy",
        ]);
        f.commands.length = 0;
        await setupBinaries(f.options);
        assert.deepEqual(f.commands, [
          { command: f.destination, args: ["--version"] },
        ]);
        assert.deepEqual(
          [...Deno.readDirSync(f.directory)].map((e) => e.name),
          [path.basename(f.destination)],
        );
      }),
  );
}

for (const failure of [
  "download",
  "archive hash",
  "extraction",
  "missing executable",
  "binary hash",
  "version",
  "execution",
]) {
  Deno.test(
    `${failure} failure preserves the previous binary and cleans only its own staging`,
    () =>
      withTempDirectory(async (root) => {
        const f = await fixture(root);
        await Deno.mkdir(f.directory, { recursive: true });
        await Deno.writeTextFile(f.destination, "previous binary");
        await Deno.mkdir(path.join(f.directory, ".extract-deno"));
        const exec: NonNullable<SetupOptions["exec"]> = async (
          command,
          args,
        ) => {
          if (command === "curl") {
            await f.exec(command, args);
            if (failure === "download") throw new Error("download failed");
            if (failure === "archive hash")
              await Deno.writeTextFile(
                args[args.indexOf("--output") + 1],
                "damaged archive",
              );
          } else if (command === "unzip") {
            if (failure === "archive hash")
              assert.fail("must not extract untrusted data");
            if (failure === "extraction") throw new Error("extraction failed");
            if (failure === "missing executable") return;
            await f.exec(command, args);
            if (failure === "binary hash")
              await Deno.writeTextFile(
                path.join(args[args.indexOf("-d") + 1], "nested", "deno"),
                "damaged executable",
              );
          } else {
            if (failure === "binary hash")
              assert.fail("must not execute untrusted data");
            if (failure === "execution") throw new Error("execution failed");
            if (failure === "version") return { stdout: "deno 2.9.4\n" };
            return f.exec(command, args);
          }
        };
        await assert.rejects(
          setupBinaries({ ...f.options, args: ["deno", "--force"], exec }),
          /failed|SHA-256 mismatch|does not contain|Expected deno/,
        );
        assert.equal(await Deno.readTextFile(f.destination), "previous binary");
        assert.deepEqual(
          [...Deno.readDirSync(f.directory)].map((e) => e.name).sort(),
          [".extract-deno", path.basename(f.destination)].sort(),
        );
      }),
  );
}

for (const name of ["deno", "rg"] as const) {
  Deno.test(`${name} stale versions and corrupt caches are replaced`, () =>
    withTempDirectory(async (root) => {
      const f = await fixture(root, name);
      await Deno.mkdir(f.directory, { recursive: true });
      await Deno.writeTextFile(f.destination, f.binary);
      await setupBinaries({
        ...f.options,
        exec: (command, args) =>
          command === f.destination
            ? Promise.resolve({
                stdout: `${name === "rg" ? "ripgrep" : name} 0.0.0\n`,
              })
            : f.exec(command, args),
      });
      assert.equal(
        f.commands.some((c) => c.command === "curl"),
        true,
      );
      await Deno.writeTextFile(f.destination, "corrupt cache");
      f.commands.length = 0;
      await setupBinaries(f.options);
      assert.equal(f.commands[0].command, "curl");
      assert.equal(await Deno.readTextFile(f.destination), f.binary);
    }),
  );
}

for (const corruptSecond of [false, true]) {
  Deno.test(
    `concurrent installs isolate staging when second is ${corruptSecond ? "corrupt" : "valid"}`,
    () =>
      withTempDirectory(async (root) => {
        const f = await fixture(root);
        await Deno.mkdir(f.directory, { recursive: true });
        await Deno.writeTextFile(f.destination, "previous binary");
        const gate = Promise.withResolvers<void>();
        const archives: string[] = [];
        const run = (corrupt: boolean) =>
          setupBinaries({
            ...f.options,
            args: ["deno", "--force"],
            exec: async (command, args) => {
              if (command !== "curl") return f.exec(command, args);
              const archivePath = args[args.indexOf("--output") + 1];
              archives.push(archivePath);
              if (archives.length === 2) {
                assert.equal(
                  await Deno.readTextFile(f.destination),
                  "previous binary",
                );
                gate.resolve();
              }
              await gate.promise;
              await Deno.writeTextFile(
                archivePath,
                corrupt ? "corrupt" : f.archive,
              );
            },
          });
        const outcomes = await Promise.allSettled([
          run(false),
          run(corruptSecond),
        ]);
        assert.deepEqual(
          outcomes.map((o) => o.status),
          ["fulfilled", corruptSecond ? "rejected" : "fulfilled"],
        );
        assert.equal(new Set(archives).size, 2);
        assert.equal(await Deno.readTextFile(f.destination), f.binary);
        assert.deepEqual(
          [...Deno.readDirSync(f.directory)].map((e) => e.name),
          [path.basename(f.destination)],
        );
      }),
  );
}

Deno.test(
  "unknown pins and conflicting versions fail without running commands",
  () =>
    withTempDirectory(async (root) => {
      const f = await fixture(root);
      await assert.rejects(
        setupBinaries({
          ...f.options,
          environment: { DENO_VERSION: "v2.9.4" },
        }),
        /DENO_VERSION must match/,
      );
      await assert.rejects(
        setupBinaries({ ...f.options, args: ["unknown"] }),
        /Unsupported binary/,
      );
      await Deno.writeTextFile(
        path.join(root, "scripts", "sidecar-checksums.json"),
        "{}",
      );
      await assert.rejects(
        setupBinaries(f.options),
        /Missing trusted SHA-256 pin/,
      );
      assert.deepEqual(f.commands, []);
    }),
);

Deno.test(
  "foreign target binaries are authenticated by their pinned content without execution",
  () =>
    withTempDirectory(async (root) => {
      const f = await fixture(root, "rg", "x86_64-pc-windows-msvc");
      await setupBinaries({ ...f.options, platform: "linux" });
      assert.deepEqual(
        f.commands.map((c) => c.command),
        ["curl", "unzip"],
      );
      f.commands.length = 0;
      await setupBinaries({ ...f.options, platform: "linux" });
      assert.deepEqual(f.commands, []);
      assert.equal(await Deno.readTextFile(f.destination), f.binary);
    }),
);
