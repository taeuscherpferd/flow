import type { SkillsService } from "../services/SkillsService.js";
import type { Tool } from "./types.js";

export function createLoadSkillTool(skillsService: SkillsService): Tool {
  return {
    name: "load_skill",
    description:
      "Load the full instructions for a named skill when it's relevant to the current task. Only the name and description of each skill are visible until loaded.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "Exact skill name as listed." },
      },
      required: ["name"],
    },
    async execute(args) {
      const name = args["name"];
      if (typeof name !== "string" || name.trim().length === 0) {
        return { ok: false, content: "Error: 'name' must be a non-empty string." };
      }

      const body = skillsService.getBody(name);
      if (body === undefined) {
        const available = skillsService.listSkills().map((s) => s.name);
        return {
          ok: false,
          content: `No skill named "${name}". Available: ${available.length > 0 ? available.join(", ") : "(none)"}`,
        };
      }

      return { ok: true, content: body };
    },
  };
}
