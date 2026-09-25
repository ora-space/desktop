import assert from "node:assert/strict";
import path from "node:path";
import { controllerArguments, initialize } from "./run-minicloud.ts";

Deno.test(
  "minicloud initialization preserves edited configuration and existing checkout files",
  async () => {
    const temporary = await Deno.makeTempDir({ prefix: "mc-" });
    const root = path.join(temporary, "dev");
    try {
      await initialize(root, "local");
      const config = path.join(root, "config", "client.json");
      await Deno.writeTextFile(config, '{"port":5190}\n');
      const marker = path.join(root, "repositories", "user-file");
      await Deno.writeTextFile(marker, "preserve");
      await Deno.chmod(root, 0o775);
      await initialize(root, "local");
      assert.equal((await Deno.stat(root)).mode! & 0o777, 0o775);
      assert.equal(await Deno.readTextFile(config), '{"port":5190}\n');
      assert.equal(await Deno.readTextFile(marker), "preserve");
      const node = JSON.parse(
        await Deno.readTextFile(path.join(root, "config", "node.json")),
      );
      assert.deepEqual(node.node, {
        home_directory: path.join(root, "node"),
        identity: { Require: "minicloud-node" },
        repositories: [],
      });
      assert.equal(node.process.host_directory, path.join(root, "p"));
      await assert.rejects(
        Deno.stat(path.join(root, "p")),
        Deno.errors.NotFound,
      );
      assert.equal((await Deno.stat(config)).mode! & 0o777, 0o600);
    } finally {
      await Deno.remove(temporary, { recursive: true });
    }
  },
);

Deno.test(
  "minicloud initialization rejects a symlink rather than modifying its target",
  async () => {
    const temporary = await Deno.makeTempDir({ prefix: "mc-" });
    try {
      const target = path.join(temporary, "outside");
      await Deno.mkdir(target, { mode: 0o700 });
      const root = path.join(temporary, "dev");
      await Deno.symlink(target, root);
      await assert.rejects(initialize(root, "local"), /private directory/);
      const entries = [];
      for await (const entry of Deno.readDir(target)) entries.push(entry.name);
      assert.deepEqual(entries, []);
    } finally {
      await Deno.remove(temporary, { recursive: true });
    }
  },
);

Deno.test(
  "minicloud initialization hosts the launcher's Node and keeps the Controller API on loopback",
  async () => {
    const temporary = await Deno.makeTempDir({ prefix: "mc-" });
    const root = path.join(temporary, "dev");
    try {
      await initialize(root, "local");
      const config = path.join(root, "config");
      const controller = JSON.parse(
        await Deno.readTextFile(path.join(config, "controller.json")),
      );
      assert.deepEqual(controller.api, { node_id: "minicloud-node" });
      assert.equal(controller.controller.controller_id, "minicloud-controller");
      assert.deepEqual(controller.controller.persistence, { kind: "sqlite" });
      assert.deepEqual(controller.controller.nodes, [
        {
          node_id: "minicloud-node",
          endpoint: {
            kind: "ipc",
            path: path.join(root, "node", "control.sock"),
          },
        },
      ]);
      assert.equal(
        controller.single_node.node_config,
        path.join(config, "node.json"),
      );
      assert.equal(
        controller.single_node.node_executable.endsWith(
          "/target/debug/ora-node",
        ),
        true,
      );
      const node = JSON.parse(
        await Deno.readTextFile(path.join(config, "node.json")),
      );
      assert.equal(
        node.control.controller_id,
        controller.controller.controller_id,
      );
      assert.deepEqual(
        node.control.listen,
        controller.controller.nodes[0].endpoint,
      );
      const client = JSON.parse(
        await Deno.readTextFile(path.join(config, "client.json")),
      );
      assert.deepEqual(client, { port: 5174, controllerPort: 4820 });
      const args = controllerArguments(path.join(config, "controller.json"), {
        kind: "sqlite",
        port: client.controllerPort,
      });
      assert.equal(args.includes("--single-node"), true);
      assert.deepEqual(
        args.slice(args.indexOf("--host"), args.indexOf("--host") + 2),
        ["--host", "127.0.0.1"],
      );
      assert.deepEqual(
        args.slice(args.indexOf("--port"), args.indexOf("--port") + 2),
        ["--port", "4820"],
      );
    } finally {
      await Deno.remove(temporary, { recursive: true });
    }
  },
);

Deno.test(
  "cloud initialization coordinates through Cloud without a JSON surface or frontend",
  async () => {
    const temporary = await Deno.makeTempDir({ prefix: "mc-" });
    const root = path.join(temporary, "dev");
    try {
      await initialize(root, "cloud");
      const config = path.join(root, "config");
      const controller = JSON.parse(
        await Deno.readTextFile(path.join(config, "controller.json")),
      );
      const node = path.join(root, "node");
      assert.deepEqual(controller, {
        controller: {
          home_directory: path.join(root, "controller"),
          persistence: {
            kind: "cloud",
            endpoint: "http://127.0.0.1:8082",
            claim_interval_ms: 500,
          },
          protected_state_directories: [node, path.join(root, "p")],
          controller_id: "minicloud-controller",
          nodes: [
            {
              node_id: "minicloud-node",
              endpoint: { kind: "ipc", path: path.join(node, "control.sock") },
            },
          ],
          session: { io_timeout_ms: 10000, query_interval_ms: 1000 },
          reconnect_ms: 1000,
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        },
        single_node: {
          node_executable: controller.single_node.node_executable,
          node_config: path.join(config, "node.json"),
          ready_timeout_ms: 30000,
          stop_timeout_ms: 30000,
        },
      });
      assert.equal(
        controller.single_node.node_executable.endsWith(
          "/target/debug/ora-node",
        ),
        true,
      );
      for (const absent of [
        path.join(config, "client.json"),
        path.join(root, "vite"),
      ]) {
        await assert.rejects(Deno.stat(absent), Deno.errors.NotFound);
      }
      assert.deepEqual(
        controllerArguments(path.join(config, "controller.json"), {
          kind: "cloud",
        }),
        ["--config", path.join(config, "controller.json"), "--single-node"],
      );
    } finally {
      await Deno.remove(temporary, { recursive: true });
    }
  },
);

Deno.test(
  "cloud initialization preserves an edited Cloud endpoint",
  async () => {
    const temporary = await Deno.makeTempDir({ prefix: "mc-" });
    const root = path.join(temporary, "dev");
    try {
      await initialize(root, "cloud");
      const config = path.join(root, "config", "controller.json");
      const edited = JSON.parse(await Deno.readTextFile(config));
      edited.controller.persistence.endpoint = "http://10.0.0.2:8082";
      await Deno.writeTextFile(config, JSON.stringify(edited));
      await initialize(root, "cloud");
      assert.deepEqual(JSON.parse(await Deno.readTextFile(config)), edited);
    } finally {
      await Deno.remove(temporary, { recursive: true });
    }
  },
);

Deno.test(
  "minicloud initialization migrates legacy IPC configuration and keeps other edits",
  async () => {
    const temporary = await Deno.makeTempDir({ prefix: "mc-" });
    const root = path.join(temporary, "dev");
    try {
      await initialize(root, "local");
      const config = path.join(root, "config");
      const socket = path.join(root, "node", "control.sock");
      const nodeFile = path.join(config, "node.json");
      const controllerFile = path.join(config, "controller.json");
      // Recreate what an older launcher wrote, plus a user edit on each file.
      const { control: _, ...node } = JSON.parse(
        await Deno.readTextFile(nodeFile),
      );
      const legacyNode = {
        ...node,
        recovery_interval_ms: 2500,
        ipc: {
          controller_id: "minicloud-controller",
          endpoint: socket,
          heartbeat_ms: 700,
          frame_timeout_ms: 9000,
        },
      };
      await Deno.writeTextFile(nodeFile, JSON.stringify(legacyNode));
      const controller = JSON.parse(await Deno.readTextFile(controllerFile));
      controller.controller.reconnect_ms = 3000;
      controller.controller.nodes = [
        { node_id: "minicloud-node", endpoint: socket },
      ];
      await Deno.writeTextFile(controllerFile, JSON.stringify(controller));

      await initialize(root, "local");

      const { ipc: __, ...kept } = legacyNode;
      assert.deepEqual(JSON.parse(await Deno.readTextFile(nodeFile)), {
        ...kept,
        control: {
          controller_id: "minicloud-controller",
          heartbeat_ms: 700,
          frame_timeout_ms: 9000,
          listen: { kind: "ipc", path: socket },
        },
      });
      const migrated = JSON.parse(await Deno.readTextFile(controllerFile));
      assert.equal(migrated.controller.reconnect_ms, 3000);
      assert.deepEqual(migrated.controller.nodes, [
        { node_id: "minicloud-node", endpoint: { kind: "ipc", path: socket } },
      ]);
      assert.equal((await Deno.stat(nodeFile)).mode! & 0o777, 0o600);
      assert.equal((await Deno.stat(controllerFile)).mode! & 0o777, 0o600);

      // A second run finds nothing legacy and rewrites nothing.
      const before = await Deno.readTextFile(nodeFile);
      await initialize(root, "local");
      assert.equal(await Deno.readTextFile(nodeFile), before);
    } finally {
      await Deno.remove(temporary, { recursive: true });
    }
  },
);
