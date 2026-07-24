export type ChatRole = "system" | "user" | "assistant" | "tool";

export type ThinkingMode =
  | "default"
  | "off"
  | "on"
  | "low"
  | "medium"
  | "high";

export interface ChatMessage {
  role: ChatRole;
  content: string;
  thinking?: string;
  toolCalls?: ToolCall[];
  toolCallId?: string;
  toolName?: string;
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: JsonObject;
}

export interface ToolDef {
  type: "function";
  function: {
    name: string;
    description: string;
    parameters: JSONSchema;
  };
}

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

export interface ChatCompletionOptions {
  numCtx?: number;
  thinking?: ThinkingMode;
}

export interface ChatCompletionRequest {
  model: string;
  messages: ChatMessage[];
  tools?: ToolDef[];
  options?: ChatCompletionOptions;
  signal?: AbortSignal;
}

export interface ChatCompletionResult {
  message: ChatMessage;
}

export interface ModelProvider {
  readonly id: string;
  chat(request: ChatCompletionRequest): Promise<ChatCompletionResult>;
}
import type { JsonObject } from "#src/workflows/types.js";
