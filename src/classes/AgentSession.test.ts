import assert from "node:assert/strict";
import test from "node:test";
import { AgentSession } from "#src/classes/AgentSession.js";
import type {
  ChatCompletionRequest,
  ChatCompletionResult,
  ModelProvider,
} from "#src/providers/types.js";
import { AgentComsService } from "#src/services/AgentComsService.js";
import { SkillsService } from "#src/services/SkillsService.js";
import { ToolRegistry } from "#src/tools/ToolRegistry.js";
import type { ToolExecutionContext } from "#src/tools/types.js";

class RecordingProvider implements ModelProvider {
  readonly id = "recording";
  readonly requests: ChatCompletionRequest[] = [];

  async chat(
    request: ChatCompletionRequest,
  ): Promise<ChatCompletionResult> {
    this.requests.push(structuredClone(request));
    return {
      message: {
        role: "assistant",
        content: "answer",
        thinking: `reasoning-${this.requests.length}`,
      },
    };
  }
}

function createSession(
  provider: RecordingProvider,
  history?: ConstructorParameters<typeof AgentComsService>[4],
): AgentSession {
  const tools = new ToolRegistry(new SkillsService("global", "project"));
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
    history ?? "system",
    context,
  );
  return new AgentSession("session", "test", service);
}

test("forwards thinking per run without making it sticky", async () => {
  const provider = new RecordingProvider();
  const session = createSession(provider);

  await session.run("deep task", { thinking: "off" });
  await session.run("default task");

  assert.equal(provider.requests[0]?.options?.thinking, "off");
  assert.equal("thinking" in provider.requests[1]!.options!, false);
});

test("copied session history retains prior thinking", async () => {
  const sourceProvider = new RecordingProvider();
  const source = createSession(sourceProvider);
  await source.run("first");

  const forkProvider = new RecordingProvider();
  const fork = createSession(forkProvider, source.snapshotHistory());
  await fork.run("second");

  assert.equal(
    forkProvider.requests[0]?.messages[2]?.thinking,
    "reasoning-1",
  );
});
