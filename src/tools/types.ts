import type { JSONSchema } from "../providers/types.js";

export interface ToolExecutionContext {
  cwd: string;
  /** Seam for future permissions.json enforcement. Stubbed to always allow today. */
  requestPermission: (
    toolName: string,
    args: Record<string, unknown>,
  ) => Promise<boolean>;
}

export interface ToolResult {
  ok: boolean;
  /** Always stringified — this is what goes back as a role:"tool" message's content. */
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
