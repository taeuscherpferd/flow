import type { JSONSchema } from "#src/providers/types.js";
import type { SecretsProvider } from "#src/services/SecretsProvider.js";
import type { JsonObject } from "#src/workflows/types.js";
import type { AgentExecutionMode } from "#src/agents/types.js";

export type ToolEffect =
  | "read"
  | "write"
  | "command"
  | "external"
  | "schedule";

export interface ToolExecutionContext {
  cwd: string;
  requestPermission: (
    toolName: string,
    args: JsonObject,
    effect?: ToolEffect,
  ) => Promise<boolean>;
  secrets: SecretsProvider;
  executionMode?: AgentExecutionMode;
}

export interface ToolResult {
  ok: boolean;
  content: string;
}

export interface Tool {
  name: string;
  description: string;
  effect?: ToolEffect;
  permissionMode?: "effect" | "self-managed";
  parameters: JSONSchema;
  execute(
    args: JsonObject,
    ctx: ToolExecutionContext,
    signal?: AbortSignal,
  ): Promise<ToolResult>;
}
