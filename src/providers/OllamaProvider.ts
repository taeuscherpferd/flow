import type {
  ChatCompletionRequest,
  ChatCompletionResult,
  ChatMessage,
  ChatRole,
  ModelProvider,
  ThinkingMode,
  ToolCall,
  ToolDef,
} from "#src/providers/types.js";
import type { JsonObject } from "#src/workflows/types.js";

interface OllamaWireToolCall {
  function: { name: string; arguments: JsonObject };
}

interface OllamaWireMessage {
  role: string;
  content: string;
  thinking?: string;
  tool_calls?: OllamaWireToolCall[];
}

interface OllamaChatRequestBody {
  model: string;
  messages: OllamaWireMessage[];
  tools?: ToolDef[];
  stream: false;
  think?: boolean | "low" | "medium" | "high";
  options?: { num_ctx: number };
}

interface OllamaChatResponse {
  message: OllamaWireMessage;
  done: boolean;
}

export class OllamaProviderError extends Error {}

let callCounter = 0;

function synthesizeToolCall(raw: OllamaWireToolCall): ToolCall {
  callCounter += 1;
  return {
    id: `call_${callCounter}_${Date.now()}`,
    name: raw.function.name,
    arguments: raw.function.arguments,
  };
}

function toWireMessage(message: ChatMessage): OllamaWireMessage {
  const wire: OllamaWireMessage = { role: message.role, content: message.content };
  if (message.thinking && message.thinking.length > 0) {
    wire.thinking = message.thinking;
  }
  if (message.toolCalls && message.toolCalls.length > 0) {
    wire.tool_calls = message.toolCalls.map((call) => ({
      function: { name: call.name, arguments: call.arguments },
    }));
  }
  return wire;
}

function fromWireMessage(message: OllamaWireMessage): ChatMessage {
  const toolCalls = message.tool_calls?.map(synthesizeToolCall);
  const result: ChatMessage = {
    role: message.role as ChatRole,
    content: message.content,
  };
  if (message.thinking && message.thinking.length > 0) {
    result.thinking = message.thinking;
  }
  if (toolCalls && toolCalls.length > 0) {
    result.toolCalls = toolCalls;
  }
  return result;
}

function toOllamaThinking(
  thinking: ThinkingMode | undefined,
): boolean | "low" | "medium" | "high" | undefined {
  switch (thinking) {
    case undefined:
    case "default":
      return undefined;
    case "off":
      return false;
    case "on":
      return true;
    case "low":
    case "medium":
    case "high":
      return thinking;
  }
}

export class OllamaProvider implements ModelProvider {
  readonly id = "ollama";

  constructor(private readonly baseUrl: string) {}

  async chat(request: ChatCompletionRequest): Promise<ChatCompletionResult> {
    const body: OllamaChatRequestBody = {
      model: request.model,
      messages: request.messages.map(toWireMessage),
      stream: false,
    };
    if (request.tools && request.tools.length > 0) {
      body.tools = request.tools;
    }
    if (request.options?.numCtx !== undefined) {
      body.options = { num_ctx: request.options.numCtx };
    }
    const thinking = toOllamaThinking(request.options?.thinking);
    if (thinking !== undefined) {
      body.think = thinking;
    }

    let response: Response;
    try {
      response = await fetch(`${this.baseUrl}/api/chat`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        ...(request.signal === undefined ? {} : { signal: request.signal }),
      });
    } catch (err) {
      throw new OllamaProviderError(
        `Could not reach Ollama at ${this.baseUrl} — is it running? (ollama serve)\n${String(err)}`,
      );
    }

    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new OllamaProviderError(`Ollama returned ${response.status}: ${text}`);
    }

    const data = (await response.json()) as OllamaChatResponse;
    return { message: fromWireMessage(data.message) };
  }
}
