import type { ChatMessage, ModelProvider, ToolCall } from "#src/providers/types.js";
import type { ToolRegistry } from "#src/tools/ToolRegistry.js";
import type { ToolExecutionContext, ToolResult } from "#src/tools/types.js";

const MAX_TOOL_ITERATIONS = 8;

export class AgentComsService {
  private readonly history: ChatMessage[] = [];

  constructor(
    private provider: ModelProvider,
    private model: string,
    private contextWindow: number,
    private readonly toolRegistry: ToolRegistry,
    systemPromptOrHistory: string | ChatMessage[],
    private readonly toolCtx: ToolExecutionContext,
  ) {
    if (typeof systemPromptOrHistory === "string") {
      this.history.push({ role: "system", content: systemPromptOrHistory });
    } else {
      this.history.push(...structuredClone(systemPromptOrHistory));
    }
  }

  getModel(): string {
    return this.model;
  }

  setModel(provider: ModelProvider, model: string, contextWindow: number): void {
    this.provider = provider;
    this.model = model;
    this.contextWindow = contextWindow;
  }

  async handleUserMessage(
    userText: string,
    signal?: AbortSignal,
    tools: "default" | "none" = "default",
  ): Promise<string> {
    signal?.throwIfAborted();
    this.history.push({ role: "user", content: userText });

    for (let i = 0; i < MAX_TOOL_ITERATIONS; i++) {
      let result;
      try {
        result = await this.provider.chat({
          model: this.model,
          messages: this.history,
          ...(tools === "default"
            ? { tools: this.toolRegistry.getToolDefs() }
            : {}),
          options: { numCtx: this.contextWindow },
          ...(signal === undefined ? {} : { signal }),
        });
      } catch (err) {
        if (signal?.aborted) throw err;
        return `Error: ${err instanceof Error ? err.message : String(err)}`;
      }

      signal?.throwIfAborted();
      this.history.push(
        tools === "none"
          ? {
              role: result.message.role,
              content: result.message.content,
            }
          : result.message,
      );

      if (
        tools === "none" ||
        !result.message.toolCalls ||
        result.message.toolCalls.length === 0
      ) {
        return result.message.content;
      }

      for (const call of result.message.toolCalls) {
        signal?.throwIfAborted();
        const toolResult = await this.executeToolCall(call, signal);
        signal?.throwIfAborted();
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

  clearHistory(systemContexts: string[] = []): void {
    this.history.length = 1;
    for (const content of systemContexts) {
      this.injectSystemContext(content);
    }
  }

  snapshotHistory(): ChatMessage[] {
    return structuredClone(this.history);
  }

  restoreHistory(history: ChatMessage[]): void {
    this.history.length = 0;
    this.history.push(...structuredClone(history));
  }

  injectSkillBody(name: string, body: string): void {
    this.history.push({
      role: "user",
      content: `[Loaded skill "${name}" per user request]\n\n${body}`,
    });
  }

  injectSystemContext(content: string): void {
    if (content.trim().length === 0) return;
    this.history.push({ role: "system", content });
  }

  private async executeToolCall(
    call: ToolCall,
    signal?: AbortSignal,
  ): Promise<ToolResult> {
    const tool = this.toolRegistry.get(call.name);
    if (!tool) {
      return { ok: false, content: `Error: no such tool "${call.name}"` };
    }

    signal?.throwIfAborted();
    const allowed = await this.toolCtx.requestPermission(call.name, call.arguments);
    signal?.throwIfAborted();
    if (!allowed) {
      return { ok: false, content: `Permission denied for tool "${call.name}".` };
    }

    try {
      return await tool.execute(call.arguments, this.toolCtx, signal);
    } catch (err) {
      if (signal?.aborted) throw err;
      return { ok: false, content: `Error executing "${call.name}": ${String(err)}` };
    }
  }
}
