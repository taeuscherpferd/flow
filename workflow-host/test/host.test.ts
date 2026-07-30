import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

interface Pending {
  resolve(value: JsonValue): void;
  reject(error: Error): void;
}

class ResponseError extends Error {
  constructor(readonly code: number, message: string) {
    super(message);
  }
}

class HostHarness {
  private readonly pending = new Map<number, Pending>();
  private nextId = 1;
  private readonly child: ChildProcessWithoutNullStreams;

  constructor() {
    this.child = spawn(
      process.execPath,
      ["--import", "tsx", path.resolve("src/index.ts")],
      { cwd: path.resolve("."), stdio: ["pipe", "pipe", "pipe"] },
    );
    const lines = createInterface({
      input: this.child.stdout,
      crlfDelay: Infinity,
    });
    lines.on("line", (line) => {
      void this.receive(JSON.parse(line) as JsonValue);
    });
  }

  request(method: string, params: JsonValue): Promise<JsonValue> {
    const id = this.nextId;
    this.nextId += 1;
    const response = new Promise<JsonValue>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.write({ jsonrpc: "2.0", id, method, params });
    return response;
  }

  async close(): Promise<void> {
    if (this.child.exitCode === null) {
      await this.request("host.shutdown", {});
      await new Promise<void>((resolve) => {
        this.child.once("exit", () => resolve());
      });
    }
  }

  kill(): void {
    this.child.kill("SIGKILL");
  }

  private async receive(message: JsonValue): Promise<void> {
    if (
      typeof message !== "object" ||
      message === null ||
      Array.isArray(message)
    ) {
      return;
    }
    const method = message["method"];
    const id = message["id"];
    if (typeof method === "string" && typeof id === "number") {
      if (method === "sdk.checkpoint") {
        const params = message["params"];
        assert.ok(
          typeof params === "object" &&
            params !== null &&
            !Array.isArray(params),
        );
        const callbackId = params["callbackId"];
        assert.equal(typeof callbackId, "string");
        const result = await this.request("callback.invoke", {
          callbackId,
          arguments: [],
        });
        this.write({ jsonrpc: "2.0", id, result });
      } else {
        this.write({
          jsonrpc: "2.0",
          id,
          error: { code: -32601, message: `Unhandled ${method}` },
        });
      }
      return;
    }
    if (typeof id !== "number") return;
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    if ("result" in message) {
      pending.resolve(message["result"] ?? null);
      return;
    }
    const error = message["error"];
    assert.ok(
      typeof error === "object" &&
        error !== null &&
        !Array.isArray(error),
    );
    pending.reject(
      new ResponseError(
        typeof error["code"] === "number" ? error["code"] : -32603,
        typeof error["message"] === "string"
          ? error["message"]
          : "Invalid error",
      ),
    );
  }

  private write(message: JsonValue): void {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }
}

const handshake = {
  protocolVersion: 1,
  clientName: "workflow-host-test",
  clientVersion: "1.0.0",
};

test("handshakes and executes a workflow through a nested Rust callback", async () => {
  const root = await mkdtemp(
    path.join(os.tmpdir(), "flowmation-host-test-"),
  );
  const workflowDir = path.join(root, "callback-example");
  await mkdir(workflowDir, { recursive: true });
  const entryPath = path.join(workflowDir, "WORKFLOW.js");
  await writeFile(
    entryPath,
    `
      import { defineWorkflow } from "flowmation/workflow";
      export default defineWorkflow({
        name: "callback-example",
        description: "Exercises the protocol",
        async run(context, input) {
          const saved = await context.checkpoint("saved", async () => ({
            input,
          }));
          return context.output.agent(saved);
        },
      });
    `,
    "utf-8",
  );
  const host = new HostHarness();
  try {
    const handshakeResult = await host.request("host.handshake", handshake);
    assert.equal(
      (handshakeResult as { protocolVersion: number }).protocolVersion,
      1,
    );

    const inspected = await host.request("workflow.inspect", { entryPath });
    assert.equal(
      (
        inspected as {
          metadata: { name: string };
        }
      ).metadata.name,
      "callback-example",
    );

    const result = await host.request("workflow.run", {
      entryPath,
      runId: "run-1",
      projectDir: root,
      input: "hello",
    });
    assert.deepEqual(result, {
      value: { input: "hello" },
      presentation: "agent",
    });
  } finally {
    await host.close().catch(() => host.kill());
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects a mismatched protocol version before loading workflows", async () => {
  const host = new HostHarness();
  try {
    await assert.rejects(
      host.request("host.handshake", {
        ...handshake,
        protocolVersion: 99,
      }),
      (error: Error) =>
        error instanceof ResponseError && error.code === -32001,
    );
    await host.request("host.handshake", handshake);
  } finally {
    await host.close().catch(() => host.kill());
  }
});

test("loads TypeScript through the virtual SDK and refreshes a portable editor path", async () => {
  const root = await mkdtemp(
    path.join(os.tmpdir(), "flowmation-host-typescript-"),
  );
  const workflowsDir = path.join(root, "workflows");
  const workflowDir = path.join(workflowsDir, "typed");
  await mkdir(workflowDir, { recursive: true });
  await writeFile(
    path.join(workflowsDir, "tsconfig.json"),
    JSON.stringify({
      compilerOptions: {
        strict: true,
        noUncheckedIndexedAccess: true,
        paths: {
          "flowmation/workflow": ["D:/stale/flowmation/sdk.ts"],
        },
      },
    }),
    "utf-8",
  );
  const entryPath = path.join(workflowDir, "WORKFLOW.ts");
  await writeFile(
    entryPath,
    `
      import { defineWorkflow } from "flowmation/workflow";
      interface Input { value: string }
      export default defineWorkflow<Input, { result: string }>({
        name: "typed",
        description: "Exercises TypeScript loading",
        input: {
          schema: {
            type: "object",
            properties: { value: { type: "string" } },
            required: ["value"],
            additionalProperties: false,
          },
        },
        async run(_context, input) {
          return { result: input.value };
        },
      });
    `,
    "utf-8",
  );

  const host = new HostHarness();
  try {
    await host.request("host.handshake", handshake);
    const inspected = await host.request("workflow.inspect", { entryPath });
    assert.equal(
      (inspected as { metadata: { name: string } }).metadata.name,
      "typed",
    );
    const result = await host.request("workflow.run", {
      entryPath,
      runId: "typed-run",
      projectDir: root,
      input: { value: "ok" },
    });
    assert.deepEqual(result, {
      value: { result: "ok" },
      presentation: "direct",
    });

    const generated = JSON.parse(
      await readFile(path.join(workflowsDir, "tsconfig.json"), "utf-8"),
    ) as {
      compilerOptions: {
        noUncheckedIndexedAccess?: boolean;
        paths: Record<string, string[]>;
      };
    };
    const sdkReference =
      generated.compilerOptions.paths["flowmation/workflow"]?.[0];
    assert.ok(sdkReference);
    assert.equal(path.isAbsolute(sdkReference), false);
    assert.equal(
      generated.compilerOptions.noUncheckedIndexedAccess,
      true,
    );
    assert.notEqual(sdkReference, "D:/stale/flowmation/sdk.ts");
  } finally {
    await host.close().catch(() => host.kill());
    await rm(root, { recursive: true, force: true });
  }
});
