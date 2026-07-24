import type { JSONSchema } from "../providers/types.js";
import type { SecretsProvider } from "../services/SecretsProvider.js";
import type { JsonObject } from "../workflows/types.js";

export interface ToolExecutionContext {
  cwd: string;
  requestPermission: (
    toolName: string,
    args: JsonObject,
  ) => Promise<boolean>;
  secrets: SecretsProvider;
}

export interface ToolResult {
  ok: boolean;
  content: string;
}

export interface Tool {
  name: string;
  description: string;
  parameters: JSONSchema;
  execute(
    args: JsonObject,
    ctx: ToolExecutionContext,
    signal?: AbortSignal,
  ): Promise<ToolResult>;
}
