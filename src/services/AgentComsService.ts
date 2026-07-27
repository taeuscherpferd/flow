import type {
  ChatMessage,
  ModelProvider,
  ThinkingMode,
  ToolCall,
} from "#src/providers/types.js";
import type { ToolRegistry } from "#src/tools/ToolRegistry.js";
import type { ToolExecutionContext, ToolResult } from "#src/tools/types.js";

const MAX_TOOL_ITERATIONS = 8;

export interface AgentTurnOptions {
  tools?: "default" | "none";
  thinking?: ThinkingMode;
}

export interface AgentComsOptions {
  onHistoryChange?(history: ChatMessage[]): void;
}

export class AgentComsService {
  private readonly history: ChatMessage[] = [];

  constructor(
    private provider: ModelProvider,
    private model: string,
    private contextWindow: number,
    private readonly toolRegistry: ToolRegistry,
    systemPromptOrHistory: string | ChatMessage[],
    private readonly toolCtx: ToolExecutionContext,
    private readonly serviceOptions: AgentComsOptions = {},
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
    turnOptions: AgentTurnOptions = {},
    signal?: AbortSignal,
  ): Promise<string> {
    signal?.throwIfAborted();
    this.history.push({ role: "user", content: userText });
    this.compactIfNeeded();
    const tools = turnOptions.tools ?? "default";

    for (let i = 0; i < MAX_TOOL_ITERATIONS; i++) {
      let result;
      try {
        result = await this.provider.chat({
          model: this.model,
          messages: this.history,
          ...(tools === "default"
            ? { tools: this.toolRegistry.getToolDefs() }
            : {}),
          options: {
            numCtx: this.contextWindow,
            ...(turnOptions.thinking === undefined
              ? {}
              : { thinking: turnOptions.thinking }),
          },
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
              ...(result.message.thinking === undefined
                ? {}
                : { thinking: result.message.thinking }),
            }
          : result.message,
      );

      if (
        tools === "none" ||
        !result.message.toolCalls ||
        result.message.toolCalls.length === 0
      ) {
        this.notifyHistoryChange();
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

    this.notifyHistoryChange();
    return "I hit my internal tool-call limit for this turn — try rephrasing or breaking the task down.";
  }

  clearHistory(systemContexts: string[] = []): void {
    this.history.length = 1;
    for (const content of systemContexts) {
      this.injectSystemContext(content);
    }
    this.notifyHistoryChange();
  }

  snapshotHistory(): ChatMessage[] {
    return structuredClone(this.history);
  }

  restoreHistory(history: ChatMessage[]): void {
    this.history.length = 0;
    this.history.push(...structuredClone(history));
  }

  replaceSystemPrompt(systemPrompt: string): void {
    const nonSystem = this.history.filter((message) => message.role !== "system");
    this.history.length = 0;
    this.history.push({ role: "system", content: systemPrompt }, ...nonSystem);
    this.compactIfNeeded();
    this.notifyHistoryChange();
  }

  injectSkillBody(name: string, body: string): void {
    this.history.push({
      role: "user",
      content: `[Loaded skill "${name}" per user request]\n\n${body}`,
    });
    this.notifyHistoryChange();
  }

  injectSystemContext(content: string): void {
    if (content.trim().length === 0) return;
    this.history.push({ role: "system", content });
    this.notifyHistoryChange();
  }

  replaceSystemContext(previous: string, next: string): void {
    const retained = this.history.filter(
      (message) =>
        !(
          message.role === "system" &&
          previous.length > 0 &&
          message.content === previous
        ),
    );
    this.history.length = 0;
    this.history.push(...retained);
    if (next.trim().length > 0) {
      this.history.push({ role: "system", content: next });
    }
    this.notifyHistoryChange();
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
    const effect = tool.effect ?? "external";
    const permissionMode = tool.permissionMode ?? "effect";
    const allowed =
      this.toolCtx.executionMode === "scheduled"
        ? effect === "read" && permissionMode === "effect"
        : permissionMode === "self-managed" || effect === "read"
          ? true
          : await this.toolCtx.requestPermission(
              call.name,
              call.arguments,
              effect,
            );
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

  private compactIfNeeded(): void {
    const estimatedTokens = this.history.reduce(
      (total, message) =>
        total +
        Math.ceil(
          (message.content.length + (message.thinking?.length ?? 0)) / 4,
        ),
      0,
    );
    if (estimatedTokens < this.contextWindow * 0.85) return;

    const systemMessages = this.history.filter(
      (message) => message.role === "system",
    );
    const conversation = this.history.filter(
      (message) => message.role !== "system",
    );
    let retainedTokens = 0;
    let start = conversation.length;
    const target = Math.floor(this.contextWindow * 0.45);
    while (start > 0 && retainedTokens < target) {
      start -= 1;
      const message = conversation[start]!;
      retainedTokens += Math.ceil(
        (message.content.length + (message.thinking?.length ?? 0)) / 4,
      );
    }
    while (start < conversation.length && conversation[start]?.role !== "user") {
      start += 1;
    }
    const removed = start;
    this.history.length = 0;
    this.history.push(
      ...systemMessages,
      {
        role: "system",
        content: `[Conversation history compacted: ${removed} older messages were removed.]`,
      },
      ...conversation.slice(start),
    );
  }

  private notifyHistoryChange(): void {
    this.serviceOptions.onHistoryChange?.(this.snapshotHistory());
  }
}
