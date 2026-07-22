import matter from "gray-matter";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import type { ConfigScalar, SkillsConfig } from "./ConfigService.js";
import type { SecretsProvider } from "./SecretsProvider.js";

export interface SkillFrontmatter {
  name: string;
  description: string;
  version?: string;
  author?: string;
  license?: string;
  metadata?: Record<string, unknown>;
}

export interface FlowmationConfigVar {
  description?: string;
  required?: boolean;
  default?: ConfigScalar;
}

export interface FlowmationSkillMeta {
  config?: Record<string, FlowmationConfigVar>;
  secrets?: string[];
}

export interface SkillRecord {
  frontmatter: SkillFrontmatter;
  body: string;
  renderedBody: string;
  expectedConfigVars?: FlowmationSkillMeta | undefined;
  dir: string;
  source: "global" | "project";
}

function hasRequiredFrontmatterFields(data: Record<string, unknown>): boolean {
  return (
    typeof data["name"] === "string" && typeof data["description"] === "string"
  );
}

function extractExpectedConfigVars(
  data: Record<string, unknown>,
): FlowmationSkillMeta | undefined {
  const metadata = data["metadata"];
  if (typeof metadata !== "object" || metadata === null) return undefined;
  const flow = (metadata as Record<string, unknown>)["flowmation"];
  if (typeof flow !== "object" || flow === null) return undefined;
  return flow as FlowmationSkillMeta;
}

function substitute(
  body: string,
  values: Record<string, ConfigScalar>,
): string {
  let out = body;
  for (const [key, value] of Object.entries(values)) {
    out = out.split(`\${${key}}`).join(String(value));
  }
  return out;
}

export class SkillsService {
  private readonly skills = new Map<string, SkillRecord>();

  constructor(
    private readonly globalDir: string,
    private readonly projectDir: string,
    private readonly skillsConfig: SkillsConfig = {},
  ) {}

  async load(): Promise<void> {
    await this.scanInto(path.join(this.globalDir, "skills"), "global");
    await this.scanInto(path.join(this.projectDir, "skills"), "project");
  }

  private async scanInto(
    skillsDir: string,
    source: "global" | "project",
  ): Promise<void> {
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
        console.warn(
          `Skipping skill "${entry.name}" — failed to parse frontmatter in ${skillFile}: ${String(err)}`,
        );
        continue;
      }

      if (!hasRequiredFrontmatterFields(data)) {
        console.warn(
          `Skipping skill "${entry.name}" — SKILL.md at ${skillFile} is missing required "name"/"description" frontmatter.`,
        );
        continue;
      }

      const frontmatter = data as unknown as SkillFrontmatter;
      const expectedConfigVars = extractExpectedConfigVars(data);
      const body = content.trim();
      const values = this.resolveValues(frontmatter.name, expectedConfigVars);
      this.warnOnMissingConfig(frontmatter.name, expectedConfigVars, values);

      this.skills.set(frontmatter.name, {
        frontmatter,
        body,
        renderedBody: substitute(body, values),
        expectedConfigVars,
        dir: skillDir,
        source,
      });
    }
  }

  private resolveValues(
    skillName: string,
    meta: FlowmationSkillMeta | undefined,
  ): Record<string, ConfigScalar> {
    const values: Record<string, ConfigScalar> = {};
    for (const [key, spec] of Object.entries(meta?.config ?? {})) {
      if (spec?.default !== undefined) values[key] = spec.default;
    }
    for (const [key, value] of Object.entries(
      this.skillsConfig[skillName] ?? {},
    )) {
      values[key] = value;
    }
    return values;
  }

  private warnOnMissingConfig(
    skillName: string,
    meta: FlowmationSkillMeta | undefined,
    values: Record<string, ConfigScalar>,
  ): void {
    for (const [key, spec] of Object.entries(meta?.config ?? {})) {
      if (spec?.required && values[key] === undefined) {
        console.warn(
          `Skill "${skillName}" needs config "${key}" — set skills.${skillName}.${key} in ` +
            `${path.join(this.projectDir, "config.json")} or ${path.join(this.globalDir, "config.json")}.`,
        );
      }
    }
  }

  validateSecrets(secrets: SecretsProvider): void {
    for (const record of this.skills.values()) {
      for (const name of record.expectedConfigVars?.secrets ?? []) {
        if (!secrets.has(name)) {
          console.warn(
            `Skill "${record.frontmatter.name}" expects secret "${name}", which is not set — ` +
              `add it to ${path.join(this.globalDir, ".env")} or your environment.`,
          );
        }
      }
    }
  }

  listSkills(): SkillFrontmatter[] {
    return Array.from(this.skills.values(), (s) => s.frontmatter);
  }

  get(name: string): SkillRecord | undefined {
    return this.skills.get(name);
  }

  getBody(name: string): string | undefined {
    return this.skills.get(name)?.renderedBody;
  }
}
