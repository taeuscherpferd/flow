import { AgentToolName, type AgentProfile } from "#src/agents/types.js";
import type { SkillFrontmatter } from "#src/services/SkillsService.js";
import path from "node:path";

export interface AgentDirectoryListing {
  name: string;
  description: string;
}

export function buildSystemPrompt(
  profile: AgentProfile,
  skills: SkillFrontmatter[],
  agents: AgentDirectoryListing[] = [],
): string {
  const sections = [profile.soul.trim()];
  if (profile.instructions.trim().length > 0) {
    sections.push(profile.instructions.trim());
  }
  if (profile.contextIndex?.trim()) {
    sections.push(`## Context Index\n\n${profile.contextIndex.trim()}`);
  }
  if (profile.contextFiles.length > 0) {
    sections.push(
      "## On-demand Context\n\n" +
        profile.contextFiles
          .map((file) =>
            `- ${
              profile.packageDirectory
                ? path.join(profile.packageDirectory, file)
                : file
            }`,
          )
          .join("\n"),
    );
  }
  if (agents.length > 0) {
    sections.push(
      "## Configured Agents\n\n" +
        agents
          .map((agent) => `- **${agent.name}**: ${agent.description}`)
          .join("\n") +
        "\n\nUse list_agents for discovery and delegate_agent with an explicit task when specialist isolation is useful.",
    );
  }
  if (skills.length > 0) {
    const loading =
      profile.tools.includes(AgentToolName.LoadSkill)
        ? "Call load_skill with a listed name to lazily load its full instructions.\n\n"
        : "";
    sections.push(
      "## Available Skills\n\n" +
        loading +
        skills
          .map((skill) => `- **${skill.name}**: ${skill.description}`)
          .join("\n"),
    );
  }
  sections.push(
    "## Tools\n\nAvailable tools: " +
      (profile.tools.length > 0 ? profile.tools.join(", ") : "(none)") +
      ".",
  );
  return sections.join("\n\n---\n\n");
}
