import type { ChatMessage, ModelProvider, ToolCall } from "../providers/types.js";
import type { ToolRegistry } from "../tools/index.js";
import type { ToolExecutionContext, ToolResult } from "../tools/types.js";

const MAX_TOOL_ITERATIONS = 8;

export class AgentComsService {
  private readonly history: ChatMessage[] = [];

  constructor(
    private readonly provider: ModelProvider,
    private readonly model: string,
    private readonly contextWindow: number,
    private readonly toolRegistry: ToolRegistry,
    systemPrompt: string,
    private readonly toolCtx: ToolExecutionContext,
  ) {
    this.history.push({ role: "system", content: systemPrompt });
  }

  async handleUserMessage(userText: string): Promise<string> {
    this.history.push({ role: "user", content: userText });

    for (let i = 0; i < MAX_TOOL_ITERATIONS; i++) {
      let result;
      try {
        result = await this.provider.chat({
          model: this.model,
          messages: this.history,
          tools: this.toolRegistry.getToolDefs(),
          options: { numCtx: this.contextWindow },
        });
      } catch (err) {
        return `Error: ${err instanceof Error ? err.message : String(err)}`;
      }

      this.history.push(result.message);

      if (!result.message.toolCalls || result.message.toolCalls.length === 0) {
        return result.message.content;
      }

      for (const call of result.message.toolCalls) {
        const toolResult = await this.executeToolCall(call);
        this.history.push({
          role: "tool",
          content: toolResult.content,
          toolCallId: call.id,
          toolName: call.name,
        });
      }
    }

    return "I hit my internal tool-call limit for this turn — try rephrasing or breaking the task down.";
  }

  /** Resets the conversation, keeping only the system prompt at index 0. */
  clearHistory(): void {
    this.history.length = 1;
  }

  /** Injects a skill's full body as context without triggering a model round-trip. */
  injectSkillBody(name: string, body: string): void {
    this.history.push({
      role: "user",
      content: `[Loaded skill "${name}" per user request]\n\n${body}`,
    });
  }

  private async executeToolCall(call: ToolCall): Promise<ToolResult> {
    const tool = this.toolRegistry.get(call.name);
    if (!tool) {
      return { ok: false, content: `Error: no such tool "${call.name}"` };
    }

    const allowed = await this.toolCtx.requestPermission(call.name, call.arguments);
    if (!allowed) {
      return { ok: false, content: `Permission denied for tool "${call.name}".` };
    }

    try {
      return await tool.execute(call.arguments, this.toolCtx);
    } catch (err) {
      return { ok: false, content: `Error executing "${call.name}": ${String(err)}` };
    }
  }
}
