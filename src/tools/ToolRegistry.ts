import type { ToolDef } from "../providers/types.js";
import type { SkillsService } from "../services/SkillsService.js";
import { createLoadSkillTool } from "./loadSkill.js";
import { readFileTool } from "./readFile.js";
import { runCommandTool } from "./runCommand.js";
import type { Tool } from "./types.js";
import { writeFileTool } from "./writeFile.js";

export class ToolRegistry {
  private readonly tools = new Map<string, Tool>();

  constructor(skillsService: SkillsService) {
    const allTools = [
      readFileTool,
      writeFileTool,
      runCommandTool,
      createLoadSkillTool(skillsService),
    ];
    for (const tool of allTools) {
      this.tools.set(tool.name, tool);
    }
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
