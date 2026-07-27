import type {
  SkillFrontmatter,
  SkillRecord,
} from "#src/services/SkillsService.js";
import type { SkillLoader } from "#src/tools/loadSkill.js";

export class AgentSkillCatalog implements SkillLoader {
  private readonly skills = new Map<string, SkillRecord>();
  private readonly ambiguousShortNames = new Set<string>();

  constructor(
    rootSkills: SkillRecord[],
    specialistSkills: Array<{ agentName: string; skills: SkillRecord[] }>,
    activeAgentName: string,
  ) {
    for (const skill of rootSkills) {
      this.skills.set(`main/${skill.frontmatter.name}`, skill);
      this.skills.set(skill.frontmatter.name, skill);
    }
    const shortOwners = new Map<string, string[]>();
    for (const specialist of specialistSkills) {
      for (const skill of specialist.skills) {
        const name = skill.frontmatter.name;
        const canonical = `${specialist.agentName}/${name}`;
        this.skills.set(canonical, skill);
        const owners = shortOwners.get(name) ?? [];
        owners.push(specialist.agentName);
        shortOwners.set(name, owners);
        if (activeAgentName === specialist.agentName) {
          this.skills.set(name, skill);
        }
      }
    }
    if (activeAgentName === "main") {
      for (const [name, owners] of shortOwners) {
        if (
          rootSkills.some((skill) => skill.frontmatter.name === name) ||
          owners.length !== 1
        ) {
          this.ambiguousShortNames.add(name);
          continue;
        }
        this.skills.set(name, this.skills.get(`${owners[0]}/${name}`)!);
      }
    }
  }

  getBody(name: string): string | undefined {
    if (this.ambiguousShortNames.has(name) && !name.includes("/")) {
      return undefined;
    }
    return this.skills.get(name)?.renderedBody;
  }

  listSkills(): SkillFrontmatter[] {
    return Array.from(this.skills)
      .filter(
        ([name]) =>
          name.includes("/") || !this.ambiguousShortNames.has(name),
      )
      .map(([name, skill]) => ({
        ...skill.frontmatter,
        name,
      }))
      .sort((left, right) => left.name.localeCompare(right.name));
  }

  listNames(): string[] {
    return this.listSkills().map((skill) => skill.name);
  }
}
