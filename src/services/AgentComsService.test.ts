import {
  ThinkingMode,
  type ChatCompletionRequest,
  type ChatCompletionResult,
  type ChatMessage,
  type ModelProvider,
} from "#src/providers/types.js";
import { AgentComsService } from "#src/services/AgentComsService.js";
import { SkillsService } from "#src/services/SkillsService.js";
import { ToolRegistry } from "#src/tools/ToolRegistry.js";
import type { Tool, ToolExecutionContext } from "#src/tools/types.js";
import assert from "node:assert/strict";
import test from "node:test";

class RecordingProvider implements ModelProvider {
  readonly id = "recording";
  readonly requests: ChatCompletionRequest[] = [];

  constructor(private readonly includeToolCall: boolean) {}

  async chat(
    request: ChatCompletionRequest,
  ): Promise<ChatCompletionResult> {
    this.requests.push(request);
    return {
      message: {
        role: "assistant",
        content: "complete",
        ...(this.includeToolCall
          ? {
              toolCalls: [
                {
                  id: "unexpected-tool-call",
                  name: "read_file",
                  arguments: { path: "README.md" },
                },
              ],
            }
          : {}),
      },
    };
  }
}

function createService(provider: ModelProvider): AgentComsService {
  const skills = new SkillsService("global", "project");
  const tools = new ToolRegistry(skills);
  const context: ToolExecutionContext = {
    cwd: process.cwd(),
    requestPermission: async () => true,
    secrets: {
      get: () => undefined,
      has: () => false,
    },
  };
  return new AgentComsService(
    provider,
    "test-model",
    8192,
    tools,
    "system",
    context,
  );
}

test("omits and ignores tools when an agent run disables them", async () => {
  const provider = new RecordingProvider(true);
  const service = createService(provider);

  const content = await service.handleUserMessage(
    "Do not use tools",
    { tools: "none" },
  );

  assert.equal(content, "complete");
  assert.equal(provider.requests.length, 1);
  assert.equal(provider.requests[0]?.tools, undefined);
  assert.deepEqual(service.snapshotHistory(), [
    { role: "system", content: "system" },
    { role: "user", content: "Do not use tools" },
    { role: "assistant", content: "complete" },
  ]);
});

test("keeps tools enabled by default", async () => {
  const provider = new RecordingProvider(false);
  const service = createService(provider);

  await service.handleUserMessage("Use tools when useful");

  assert.ok((provider.requests[0]?.tools?.length ?? 0) > 0);
});

test("applies thinking to every request in a tool loop and retains history", async () => {
  const requests: ChatCompletionRequest[] = [];
  const responses: ChatMessage[] = [
    {
      role: "assistant",
      content: "",
      thinking: "I should use a tool",
      toolCalls: [
        {
          id: "call-1",
          name: "missing-tool",
          arguments: {},
        },
      ],
    },
    {
      role: "assistant",
      content: "complete",
      thinking: "I can now answer",
    },
  ];
  const provider: ModelProvider = {
    id: "tool-loop",
    async chat(request) {
      requests.push(structuredClone(request));
      const message = responses.shift();
      assert.ok(message);
      return { message };
    },
  };
  const service = createService(provider);

  const content = await service.handleUserMessage("Use a tool", {
    thinking: ThinkingMode.High,
  });

  assert.equal(content, "complete");
  assert.equal(requests.length, 2);
  assert.deepEqual(
    requests.map((request) => request.options?.thinking),
    [ThinkingMode.High, ThinkingMode.High],
  );
  assert.equal(requests[1]?.messages[2]?.thinking, "I should use a tool");
  assert.equal(
    service.snapshotHistory().at(-1)?.thinking,
    "I can now answer",
  );
});

test("retains thinking in tool-free history", async () => {
  const provider: ModelProvider = {
    id: "thinking",
    async chat() {
      return {
        message: {
          role: "assistant",
          content: "complete",
          thinking: "private reasoning",
          toolCalls: [
            {
              id: "ignored",
              name: "read_file",
              arguments: {},
            },
          ],
        },
      };
    },
  };
  const service = createService(provider);

  await service.handleUserMessage("Answer directly", { tools: "none" });

  assert.deepEqual(service.snapshotHistory().at(-1), {
    role: "assistant",
    content: "complete",
    thinking: "private reasoning",
  });
});

test("omits thinking from provider options by default", async () => {
  const provider = new RecordingProvider(false);
  const service = createService(provider);

  await service.handleUserMessage("Use provider defaults");

  assert.equal("thinking" in provider.requests[0]!.options!, false);
});

test("clears conversation context while restoring static system context", () => {
  const provider = new RecordingProvider(false);
  const service = createService(provider);
  const workflowContext = "Available workflows: review-change";

  service.injectSystemContext(workflowContext);
  service.injectSkillBody("temporary-skill", "Temporary skill instructions");
  service.clearHistory([workflowContext]);

  assert.deepEqual(service.snapshotHistory(), [
    { role: "system", content: "system" },
    { role: "system", content: workflowContext },
  ]);
});

test("aborts an active tool and skips remaining tool calls", async () => {
  const controller = new AbortController();
  const toolStarted = Promise.withResolvers<void>();
  let skippedToolCalls = 0;
  const provider: ModelProvider = {
    id: "tool-calling",
    async chat() {
      return {
        message: {
          role: "assistant",
          content: "",
          toolCalls: [
            {
              id: "blocking-call",
              name: "blocking",
              arguments: {},
            },
            {
              id: "skipped-call",
              name: "skipped",
              arguments: {},
            },
          ],
        },
      };
    },
  };
  const skills = new SkillsService("global", "project");
  const tools = new ToolRegistry(skills);
  const blockingTool: Tool = {
    name: "blocking",
    description: "Waits until its workflow is cancelled.",
    parameters: { type: "object", properties: {} },
    async execute(_args, _context, signal) {
      assert.ok(signal);
      toolStarted.resolve();
      await new Promise<void>((_resolve, reject) => {
        signal.addEventListener(
          "abort",
          () => reject(new Error("tool aborted")),
          { once: true },
        );
      });
      return { ok: true, content: "unreachable" };
    },
  };
  const skippedTool: Tool = {
    name: "skipped",
    description: "Must not execute after cancellation.",
    parameters: { type: "object", properties: {} },
    async execute() {
      skippedToolCalls += 1;
      return { ok: true, content: "unexpected" };
    },
  };
  tools.register(blockingTool);
  tools.register(skippedTool);
  const context: ToolExecutionContext = {
    cwd: process.cwd(),
    requestPermission: async () => true,
    secrets: {
      get: () => undefined,
      has: () => false,
    },
  };
  const service = new AgentComsService(
    provider,
    "test-model",
    8192,
    tools,
    "system",
    context,
  );

  const execution = service.handleUserMessage(
    "Run both tools",
    {},
    controller.signal,
  );
  await toolStarted.promise;
  controller.abort();

  await assert.rejects(execution, /tool aborted/);
  assert.equal(skippedToolCalls, 0);
});

test("automatically allows reads and denies scheduled effects", async () => {
  const requests: ChatCompletionRequest[] = [];
  const calls: ChatMessage[] = [
    {
      role: "assistant",
      content: "",
      toolCalls: [
        { id: "read", name: "safe-read", arguments: {} },
        { id: "write", name: "effectful-write", arguments: {} },
        {
          id: "workflow",
          name: "policy-managed-workflow",
          arguments: {},
        },
      ],
    },
    { role: "assistant", content: "done" },
  ];
  const provider: ModelProvider = {
    id: "permissions",
    async chat(request) {
      requests.push(structuredClone(request));
      return { message: calls.shift()! };
    },
  };
  const tools = new ToolRegistry(new SkillsService("global", "project"));
  let reads = 0;
  let writes = 0;
  let workflows = 0;
  tools.register({
    name: "safe-read",
    effect: "read",
    description: "Reads",
    parameters: { type: "object", properties: {} },
    async execute() {
      reads += 1;
      return { ok: true, content: "read" };
    },
  });
  tools.register({
    name: "effectful-write",
    effect: "write",
    description: "Writes",
    parameters: { type: "object", properties: {} },
    async execute() {
      writes += 1;
      return { ok: true, content: "write" };
    },
  });
  tools.register({
    name: "policy-managed-workflow",
    effect: "external",
    permissionMode: "self-managed",
    description: "Uses its own authorization policy",
    parameters: { type: "object", properties: {} },
    async execute() {
      workflows += 1;
      return { ok: true, content: "workflow" };
    },
  });
  const service = new AgentComsService(
    provider,
    "test",
    8192,
    tools,
    "system",
    {
      cwd: process.cwd(),
      requestPermission: async () => true,
      secrets: { get: () => undefined, has: () => false },
      executionMode: "scheduled",
    },
  );

  await service.handleUserMessage("run");

  assert.equal(reads, 1);
  assert.equal(writes, 0);
  assert.equal(workflows, 0);
  assert.match(requests[1]?.messages.at(-1)?.content ?? "", /Permission denied/);
});

test("lets self-managed tools authorize themselves outside scheduled runs", async () => {
  const calls: ChatMessage[] = [
    {
      role: "assistant",
      content: "",
      toolCalls: [
        { id: "workflow", name: "policy-managed-workflow", arguments: {} },
      ],
    },
    { role: "assistant", content: "done" },
  ];
  const provider: ModelProvider = {
    id: "self-managed-permission",
    async chat() {
      return { message: calls.shift()! };
    },
  };
  const tools = new ToolRegistry(new SkillsService("global", "project"));
  let executions = 0;
  tools.register({
    name: "policy-managed-workflow",
    effect: "external",
    permissionMode: "self-managed",
    description: "Uses its own authorization policy",
    parameters: { type: "object", properties: {} },
    async execute() {
      executions += 1;
      return { ok: true, content: "workflow result" };
    },
  });
  let permissionRequests = 0;
  const service = new AgentComsService(
    provider,
    "test",
    8192,
    tools,
    "system",
    {
      cwd: process.cwd(),
      requestPermission: async () => {
        permissionRequests += 1;
        return false;
      },
      secrets: { get: () => undefined, has: () => false },
      executionMode: "direct",
    },
  );

  assert.equal(await service.handleUserMessage("run"), "done");
  assert.equal(executions, 1);
  assert.equal(permissionRequests, 0);
});
