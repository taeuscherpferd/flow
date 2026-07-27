import type { ToolDef } from "#src/providers/types.js";
import { createLoadSkillTool, type SkillLoader } from "#src/tools/loadSkill.js";
import { readFileTool } from "#src/tools/readFile.js";
import { runCommandTool } from "#src/tools/runCommand.js";
import type { Tool } from "#src/tools/types.js";
import { writeFileTool } from "#src/tools/writeFile.js";

export class ToolRegistry {
  private readonly tools = new Map<string, Tool>();
  private readonly allowlist: Set<string> | undefined;

  constructor(skillsService: SkillLoader, allowlist?: readonly string[]) {
    this.allowlist =
      allowlist === undefined ? undefined : new Set(allowlist);
    const allTools = [
      readFileTool,
      writeFileTool,
      runCommandTool,
      createLoadSkillTool(skillsService),
    ];
    for (const tool of allTools) {
      this.register(tool);
    }
  }

  register(tool: Tool): void {
    if (this.allowlist && !this.allowlist.has(tool.name)) return;
    this.tools.set(tool.name, tool);
  }

  get(name: string): Tool | undefined {
    return this.tools.get(name);
  }

  getToolDefs(): ToolDef[] {
    return Array.from(this.tools.values(), (tool) => ({
      type: "function",
      function: {
        name: tool.name,
        description: tool.description,
        parameters: tool.parameters,
      },
    }));
  }
}
