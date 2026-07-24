import assert from "node:assert/strict";
import test from "node:test";
import type {
  ChatCompletionRequest,
  ChatCompletionResult,
  ModelProvider,
} from "#src/providers/types.js";
import { ToolRegistry } from "#src/tools/ToolRegistry.js";
import type { Tool, ToolExecutionContext } from "#src/tools/types.js";
import { SkillsService } from "#src/services/SkillsService.js";
import { AgentComsService } from "#src/services/AgentComsService.js";

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
    undefined,
    "none",
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
    controller.signal,
  );
  await toolStarted.promise;
  controller.abort();

  await assert.rejects(execution, /tool aborted/);
  assert.equal(skippedToolCalls, 0);
});
