import type { JSONSchema } from "../providers/types.js";
import type { SecretsProvider } from "../services/SecretsProvider.js";

export interface ToolExecutionContext {
  cwd: string;
  requestPermission: (
    toolName: string,
    args: Record<string, unknown>,
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
    args: Record<string, unknown>,
    ctx: ToolExecutionContext,
  ): Promise<ToolResult>;
}
