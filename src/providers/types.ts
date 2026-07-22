export type ChatRole = "system" | "user" | "assistant" | "tool";

export interface ChatMessage {
  role: ChatRole;
  content: string;
  /** Only present on assistant messages that requested tool execution. */
  toolCalls?: ToolCall[];
  /** Only present on role:"tool" messages — which call this result answers. */
  toolCallId?: string;
  /** Only present on role:"tool" messages — echoes the tool name for readability/logging. */
  toolName?: string;
}

export interface ToolCall {
  /** Synthesized locally so calls and results can be paired — Ollama's wire format doesn't include one. */
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface ToolDef {
  type: "function";
  function: {
    name: string;
    description: string;
    parameters: JSONSchema;
  };
}

/** Minimal structural JSON Schema type — enough for tool params, not a full JSON Schema implementation. */
export interface JSONSchema {
  type: "object";
  properties: Record<string, JSONSchemaProperty>;
  required?: string[];
}

export interface JSONSchemaProperty {
  type: "string" | "number" | "boolean" | "object" | "array";
  description?: string;
  enum?: string[];
  items?: JSONSchemaProperty;
}

export interface ChatCompletionRequest {
  model: string;
  messages: ChatMessage[];
  tools?: ToolDef[];
  options?: { numCtx?: number };
}

export interface ChatCompletionResult {
  message: ChatMessage;
}

export interface ModelProvider {
  readonly id: string;
  chat(request: ChatCompletionRequest): Promise<ChatCompletionResult>;
}
