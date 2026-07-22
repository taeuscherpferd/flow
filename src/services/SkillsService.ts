import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import matter from "gray-matter";

export interface SkillFrontmatter {
  name: string;
  description: string;
  version?: string;
  author?: string;
  license?: string;
  metadata?: Record<string, unknown>;
}

export interface SkillRecord {
  frontmatter: SkillFrontmatter;
  body: string;
  dir: string;
  source: "global" | "project";
}

function hasRequiredFrontmatterFields(data: Record<string, unknown>): boolean {
  return typeof data["name"] === "string" && typeof data["description"] === "string";
}

export class SkillsService {
  private readonly skills = new Map<string, SkillRecord>();

  constructor(
    private readonly globalDir: string,
    private readonly projectDir: string,
  ) {}

  async load(): Promise<void> {
    await this.scanInto(path.join(this.globalDir, "skills"), "global");
    await this.scanInto(path.join(this.projectDir, "skills"), "project");
  }

  private async scanInto(skillsDir: string, source: "global" | "project"): Promise<void> {
    let entries;
    try {
      entries = await readdir(skillsDir, { withFileTypes: true });
    } catch {
      return; // no skills directory here — nothing to load
    }

    for (const entry of entries) {
      if (!entry.isDirectory()) continue;

      const skillDir = path.join(skillsDir, entry.name);
      const skillFile = path.join(skillDir, "SKILL.md");

      let raw: string;
      try {
        raw = await readFile(skillFile, "utf-8");
      } catch {
        continue; // not every subdirectory has to be a skill
      }

      let data: Record<string, unknown>;
      let content: string;
      try {
        const parsed = matter(raw);
        data = parsed.data;
        content = parsed.content;
      } catch (err) {
        console.warn(`Skipping skill "${entry.name}" — failed to parse frontmatter in ${skillFile}: ${String(err)}`);
        continue;
      }

      if (!hasRequiredFrontmatterFields(data)) {
        console.warn(
          `Skipping skill "${entry.name}" — SKILL.md at ${skillFile} is missing required "name"/"description" frontmatter.`,
        );
        continue;
      }

      const frontmatter = data as unknown as SkillFrontmatter;
      this.skills.set(frontmatter.name, {
        frontmatter,
        body: content.trim(),
        dir: skillDir,
        source,
      });
    }
  }

  listSkills(): SkillFrontmatter[] {
    return Array.from(this.skills.values(), (s) => s.frontmatter);
  }

  get(name: string): SkillRecord | undefined {
    return this.skills.get(name);
  }

  getBody(name: string): string | undefined {
    return this.skills.get(name)?.body;
  }
}
