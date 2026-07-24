import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import type {
  ChatCompletionRequest,
  ChatCompletionResult,
  ChatMessage,
  ModelProvider,
} from "#src/providers/types.js";
import type {
  WorkflowAgentRuntime,
  WorkflowAgentSessionRuntime,
} from "#src/workflows/WorkflowAgentCoordinator.js";
import { WorkflowEngine } from "#src/workflows/WorkflowEngine.js";
import { WorkflowRegistry } from "#src/workflows/WorkflowRegistry.js";
import { WorkflowRunStore } from "#src/workflows/WorkflowRunStore.js";
import type {
  WorkflowAgentRunOptions,
  WorkflowHumanAdapter,
} from "#src/workflows/types.js";

const fakeProvider: ModelProvider = {
  id: "fake",
  async chat(
    _request: ChatCompletionRequest,
  ): Promise<ChatCompletionResult> {
    throw new Error("Fake sessions handle responses directly.");
  },
};

class FakeSession implements WorkflowAgentSessionRuntime {
  private history: ChatMessage[];
  private provider = "test";
  private modelName: string;

  constructor(
    readonly id: string,
    model: string,
    private readonly onRun: (options?: WorkflowAgentRunOptions) => void,
    history: ChatMessage[] = [{ role: "system", content: "test" }],
  ) {
    this.modelName = model.includes("/")
      ? model.slice(model.indexOf("/") + 1)
      : model;
    this.history = structuredClone(history);
  }

  getModel(): { provider: string; model: string } {
    return { provider: this.provider, model: this.modelName };
  }

  async run(
    prompt: string,
    options?: WorkflowAgentRunOptions,
  ): Promise<string> {
    this.onRun(options);
    const content = `${this.modelName}:${prompt}`;
    this.history.push({ role: "user", content: prompt });
    this.history.push({ role: "assistant", content });
    return content;
  }

  snapshotHistory(): ChatMessage[] {
    return structuredClone(this.history);
  }

  restoreHistory(history: ChatMessage[]): void {
    this.history = structuredClone(history);
  }

  retarget(
    providerName: string,
    _provider: ModelProvider,
    model: string,
  ): void {
    this.provider = providerName;
    this.modelName = model;
  }
}

class FakeAgentRuntime implements WorkflowAgentRuntime {
  private nextId = 1;
  runCount = 0;
  readonly runOptions: Array<WorkflowAgentRunOptions | undefined> = [];

  createSession(
    modelSpec: string,
    history?: ChatMessage[],
    sessionId?: string,
  ): WorkflowAgentSessionRuntime {
    return new FakeSession(
      sessionId ?? `session-${this.nextId++}`,
      modelSpec,
      (options) => {
        this.runCount += 1;
        this.runOptions.push(options);
      },
      history,
    );
  }

  forkSession(
    session: WorkflowAgentSessionRuntime,
    modelSpec?: string,
  ): WorkflowAgentSessionRuntime {
    const current = session.getModel();
    return this.createSession(
      modelSpec ?? `${current.provider}/${current.model}`,
      session.snapshotHistory(),
    );
  }

  retargetSession(
    session: WorkflowAgentSessionRuntime,
    modelSpec: string,
  ): void {
    const separator = modelSpec.indexOf("/");
    const provider = separator === -1 ? "test" : modelSpec.slice(0, separator);
    const model = separator === -1 ? modelSpec : modelSpec.slice(separator + 1);
    session.retarget(provider, fakeProvider, model, 8_192);
  }
}

interface TestRuntime {
  root: string;
  globalDir: string;
  registry: WorkflowRegistry;
  store: WorkflowRunStore;
  agent: FakeAgentRuntime;
  engine: WorkflowEngine;
}

interface WorkflowTestGlobals {
  __flowCheckpointCalls?: number;
  __flowEffectCalls?: number;
  __flowMapActive?: number;
  __flowMapMaximum?: number;
}

const workflowGlobals = globalThis as typeof globalThis & WorkflowTestGlobals;

async function createRuntime(): Promise<TestRuntime> {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-engine-"));
  const globalDir = path.join(root, "global");
  const projectConfigDir = path.join(root, "project-config");
  const projectDir = path.join(root, "project");
  await mkdir(projectDir, { recursive: true });
  const registry = new WorkflowRegistry({
    globalDir,
    projectDir: projectConfigDir,
  });
  await registry.load();
  const store = new WorkflowRunStore(globalDir);
  const agent = new FakeAgentRuntime();
  const engine = new WorkflowEngine(
    agent,
    registry,
    store,
    projectDir,
  );
  return { root, globalDir, registry, store, agent, engine };
}

async function writeWorkflow(
  runtime: TestRuntime,
  name: string,
  body: string,
): Promise<void> {
  const workflowDir = path.join(
    runtime.globalDir,
    "workflows",
    name,
  );
  await mkdir(workflowDir, { recursive: true });
  await writeFile(
    path.join(workflowDir, "WORKFLOW.js"),
    `
      import { defineWorkflow } from "flowmation/workflow";
      export default defineWorkflow({
        name: "${name}",
        description: "Test workflow ${name}",
        ${body}
      });
    `,
    "utf-8",
  );
  await runtime.registry.load();
}

async function dispose(runtime: TestRuntime): Promise<void> {
  await runtime.engine.shutdown();
  runtime.store.close();
  await rm(runtime.root, { recursive: true, force: true });
}

const textResponse = (response: string): WorkflowHumanAdapter => ({
  request: async () => response,
});

test("checkpoints and human responses survive a resumed run", async () => {
  const runtime = await createRuntime();
  workflowGlobals.__flowCheckpointCalls = 0;

  try {
    await writeWorkflow(
      runtime,
      "checkpointed",
      `async run(context) {
        const draft = await context.checkpoint("draft", async () => {
          globalThis.__flowCheckpointCalls += 1;
          return { title: "Saved draft" };
        });
        const answer = await context.human.ask({ prompt: "Continue?" });
        return { draft, answer };
      }`,
    );

    const waiting = await runtime.engine.start("checkpointed", "");
    assert.equal(waiting.run.status, "waiting");
    assert.equal(workflowGlobals.__flowCheckpointCalls, 1);

    const completed = await runtime.engine.resume(waiting.run.id, {
      humanAdapter: textResponse("yes"),
    });
    assert.equal(completed.run.status, "completed");
    assert.deepEqual(completed.value, {
      draft: { title: "Saved draft" },
      answer: "yes",
    });
    assert.equal(workflowGlobals.__flowCheckpointCalls, 1);
  } finally {
    delete workflowGlobals.__flowCheckpointCalls;
    await dispose(runtime);
  }
});

test("ordinary agent calls rerun unless wrapped in a checkpoint", async () => {
  const runtime = await createRuntime();

  try {
    await writeWorkflow(
      runtime,
      "straight-through",
      `async run(context) {
        const agent = await context.agents.create({ model: "small" });
        const response = await agent.run("draft");
        await context.human.ask({ prompt: "Continue?" });
        return response.content;
      }`,
    );

    const waiting = await runtime.engine.start("straight-through", "");
    assert.equal(waiting.run.status, "waiting");
    assert.equal(runtime.agent.runCount, 1);
    await runtime.engine.resume(waiting.run.id, {
      humanAdapter: textResponse("yes"),
    });
    assert.equal(runtime.agent.runCount, 2);
  } finally {
    await dispose(runtime);
  }
});

test("forwards workflow agent thinking options to the runtime session", async () => {
  const runtime = await createRuntime();

  try {
    await writeWorkflow(
      runtime,
      "thinking",
      `async run(context) {
        const agent = await context.agents.create({ model: "small" });
        const response = await agent.run("review", {
          tools: "none",
          thinking: "high",
        });
        return response.content;
      }`,
    );

    const result = await runtime.engine.start("thinking", "");

    assert.equal(result.run.status, "completed");
    assert.deepEqual(runtime.agent.runOptions, [
      { tools: "none", thinking: "high" },
    ]);
  } finally {
    await dispose(runtime);
  }
});

test("scopes elevation thinking to runs inside the operation", async () => {
  const runtime = await createRuntime();

  try {
    await writeWorkflow(
      runtime,
      "elevation-thinking",
      `async run(context) {
        const agent = await context.agents.create({ model: "small" });
        await context.elevate({
          model: "reviewer",
          thinking: "high",
          attempts: 1,
          context: { mode: "reuse", session: agent },
          operation: async ({ session }) => {
            await session.run("elevated review");
            return (await session.run("fast verification", {
              thinking: "off",
            })).content;
          },
          check: () => true,
        });
        await agent.run("ordinary follow-up");
        return "complete";
      }`,
    );

    const result = await runtime.engine.start("elevation-thinking", "");

    assert.equal(result.run.status, "completed");
    assert.deepEqual(runtime.agent.runOptions, [
      { tools: "default", thinking: "high" },
      { tools: "default", thinking: "off" },
      { tools: "default" },
    ]);
  } finally {
    await dispose(runtime);
  }
});

test("completed effects are reused when a run resumes", async () => {
  const runtime = await createRuntime();
  workflowGlobals.__flowEffectCalls = 0;

  try {
    await writeWorkflow(
      runtime,
      "effect",
      `async run(context) {
        const effect = await context.effect("publish", {
          idempotencyKey: "branch-123",
          run: async ({ idempotencyKey }) => {
            globalThis.__flowEffectCalls += 1;
            return { idempotencyKey };
          },
        });
        await context.human.ask({ prompt: "Finish?" });
        return effect;
      }`,
    );

    const waiting = await runtime.engine.start("effect", "");
    assert.equal(waiting.run.status, "waiting");
    const completed = await runtime.engine.resume(waiting.run.id, {
      humanAdapter: textResponse("yes"),
    });
    assert.equal(completed.run.status, "completed");
    assert.deepEqual(completed.value, { idempotencyKey: "branch-123" });
    assert.equal(workflowGlobals.__flowEffectCalls, 1);
  } finally {
    delete workflowGlobals.__flowEffectCalls;
    await dispose(runtime);
  }
});

test("resume rejects changes in the workflow directory", async () => {
  const runtime = await createRuntime();
  const workflowDir = path.join(
    runtime.globalDir,
    "workflows",
    "dependent-resume",
  );
  const helperPath = path.join(workflowDir, "value.js");

  try {
    await mkdir(workflowDir, { recursive: true });
    await writeFile(helperPath, 'export const value = "first";', "utf-8");
    await writeFile(
      path.join(workflowDir, "WORKFLOW.js"),
      `
        import { defineWorkflow } from "flowmation/workflow";
        import { value } from "./value.js";
        export default defineWorkflow({
          name: "dependent-resume",
          description: "Waits before returning a dependency",
          async run(context) {
            await context.human.ask({ prompt: "Continue?" });
            return value;
          },
        });
      `,
      "utf-8",
    );
    await runtime.registry.load();

    const waiting = await runtime.engine.start("dependent-resume", "");
    assert.equal(waiting.run.status, "waiting");
    await writeFile(helperPath, 'export const value = "second";', "utf-8");

    const resumed = await runtime.engine.resume(waiting.run.id, {
      humanAdapter: textResponse("yes"),
    });
    assert.equal(resumed.run.status, "version-mismatch");
  } finally {
    await dispose(runtime);
  }
});

test("map limits concurrency and preserves result order", async () => {
  const runtime = await createRuntime();
  workflowGlobals.__flowMapActive = 0;
  workflowGlobals.__flowMapMaximum = 0;

  try {
    await writeWorkflow(
      runtime,
      "mapped",
      `async run(context) {
        return context.map([1, 2, 3, 4], {
          concurrency: 2,
          run: async (value) => {
            globalThis.__flowMapActive += 1;
            globalThis.__flowMapMaximum = Math.max(
              globalThis.__flowMapMaximum,
              globalThis.__flowMapActive,
            );
            await new Promise((resolve) => setImmediate(resolve));
            globalThis.__flowMapActive -= 1;
            return value * 2;
          },
        });
      }`,
    );

    const completed = await runtime.engine.start("mapped", "");
    assert.deepEqual(completed.value, [2, 4, 6, 8]);
    assert.equal(workflowGlobals.__flowMapMaximum, 2);
  } finally {
    delete workflowGlobals.__flowMapActive;
    delete workflowGlobals.__flowMapMaximum;
    await dispose(runtime);
  }
});

test("exec captures output and rejects failed commands", async () => {
  const runtime = await createRuntime();

  try {
    await writeWorkflow(
      runtime,
      "exec",
      `async run(context) {
        const success = await context.exec(process.execPath, [
          "-e",
          "process.stdout.write('hello')",
        ]);
        const failure = await context.exec(process.execPath, [
          "-e",
          "process.stderr.write('nope'); process.exit(2)",
        ], { allowFailure: true });
        return {
          stdout: success.stdout,
          failureCode: failure.exitCode,
          stderr: failure.stderr,
        };
      }`,
    );

    const completed = await runtime.engine.start("exec", "");
    assert.deepEqual(completed.value, {
      stdout: "hello",
      failureCode: 2,
      stderr: "nope",
    });
  } finally {
    await dispose(runtime);
  }
});
