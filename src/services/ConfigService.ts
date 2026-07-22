import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

export interface ProviderConfig {
  baseUrl: string;
  models: Array<{ name: string; contextWindow: number }>;
}

export interface ModelsConfig {
  defaultProvider: string;
  defaultModel: string;
  providers: Record<string, ProviderConfig>;
}

export type ConfigScalar = string | number | boolean;

export type SkillsConfig = Record<string, Record<string, ConfigScalar>>;

export interface AppConfig {
  skills?: SkillsConfig;
}

export interface ResolvedConfig {
  models: ModelsConfig;
  skillsConfig: SkillsConfig;
  soul: string;
  agentsInstructions: string;
  globalDir: string;
  projectDir: string;
}

export class ConfigError extends Error {}

const DEFAULT_SOUL = "You are a helpful, terse coding assistant.\n";

const DEFAULT_MODELS_CONFIG: ModelsConfig = {
  defaultProvider: "ollama",
  defaultModel: "llama3.1",
  providers: {
    ollama: { baseUrl: "http://localhost:11434", models: [] },
  },
};

async function pathExists(target: string): Promise<boolean> {
  try {
    await access(target);
    return true;
  } catch {
    return false;
  }
}

async function readIfExists(target: string): Promise<string | undefined> {
  try {
    return await readFile(target, "utf-8");
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw err;
  }
}

async function readJsonIfExists<T>(target: string): Promise<T | undefined> {
  const raw = await readIfExists(target);
  if (raw === undefined) return undefined;
  try {
    return JSON.parse(raw) as T;
  } catch (err) {
    throw new ConfigError(`Failed to parse ${target}: ${String(err)}`);
  }
}

export class ConfigService {
  readonly globalDir: string;
  readonly projectDir: string;

  constructor() {
    this.globalDir = path.join(os.homedir(), ".work-agent");
    this.projectDir = path.join(process.cwd(), ".work-agent");
  }

  async load(): Promise<ResolvedConfig> {
    await this.ensureGlobalScaffold();

    const globalModels =
      (await readJsonIfExists<ModelsConfig>(
        path.join(this.globalDir, "models.json"),
      )) ?? DEFAULT_MODELS_CONFIG;
    const projectModels = await readJsonIfExists<Partial<ModelsConfig>>(
      path.join(this.projectDir, "models.json"),
    );
    const models = this.mergeModelsConfig(globalModels, projectModels);
    this.validateModelsConfig(models);

    const globalApp =
      (await readJsonIfExists<AppConfig>(
        path.join(this.globalDir, "config.json"),
      )) ?? {};
    const projectApp =
      (await readJsonIfExists<AppConfig>(
        path.join(this.projectDir, "config.json"),
      )) ?? {};
    const skillsConfig = this.mergeSkillsConfig(
      globalApp.skills,
      projectApp.skills,
    );

    const soul = await this.loadSoul();
    const agentsInstructions = await this.loadAgentsInstructions();

    return {
      models,
      skillsConfig,
      soul,
      agentsInstructions,
      globalDir: this.globalDir,
      projectDir: this.projectDir,
    };
  }

  private mergeSkillsConfig(
    global: SkillsConfig | undefined,
    project: SkillsConfig | undefined,
  ): SkillsConfig {
    const merged: SkillsConfig = {};
    const names = new Set([
      ...Object.keys(global ?? {}),
      ...Object.keys(project ?? {}),
    ]);
    for (const name of names) {
      merged[name] = { ...global?.[name], ...project?.[name] };
    }
    return merged;
  }

  private async ensureGlobalScaffold(): Promise<void> {
    if (await pathExists(this.globalDir)) return;

    await mkdir(path.join(this.globalDir, "skills"), { recursive: true });
    await writeFile(
      path.join(this.globalDir, "models.json"),
      JSON.stringify(DEFAULT_MODELS_CONFIG, null, 2),
      "utf-8",
    );

    await writeFile(
      path.join(this.globalDir, "config.json"),
      JSON.stringify({ skills: {} }, null, 2),
      "utf-8",
    );

    await writeFile(
      path.join(this.globalDir, "SOUL.md"),
      DEFAULT_SOUL,
      "utf-8",
    );

    await writeFile(path.join(this.globalDir, "AGENTS.md"), "", "utf-8");

    console.log(
      `First run: created ${this.globalDir} with defaults. Edit it any time.`,
    );
  }

  private mergeModelsConfig(
    global: ModelsConfig,
    project: Partial<ModelsConfig> | undefined,
  ): ModelsConfig {
    if (!project) return global;

    const providers: Record<string, ProviderConfig> = {
      ...global.providers,
      ...(project.providers ?? {}),
    };

    return {
      defaultProvider: project.defaultProvider ?? global.defaultProvider,
      defaultModel: project.defaultModel ?? global.defaultModel,
      providers,
    };
  }

  private validateModelsConfig(models: ModelsConfig): void {
    const provider = models.providers[models.defaultProvider];
    if (!provider) {
      throw new ConfigError(
        `No provider named "${models.defaultProvider}" configured. Check models.json in ${this.globalDir} or ${this.projectDir}.`,
      );
    }

    const hasModel = provider.models.some(
      (m) => m.name === models.defaultModel,
    );

    if (!hasModel) {
      throw new ConfigError(
        `No model configured for "${models.defaultProvider}". Pull one (e.g. "ollama pull ${models.defaultModel}") ` +
          `and add it to models.json, or update defaultModel in ${path.join(this.globalDir, "models.json")}.`,
      );
    }
  }

  private async loadSoul(): Promise<string> {
    const project = await readIfExists(path.join(this.projectDir, "SOUL.md"));
    if (project !== undefined) return project;
    const global = await readIfExists(path.join(this.globalDir, "SOUL.md"));
    return global ?? DEFAULT_SOUL;
  }

  private async loadAgentsInstructions(): Promise<string> {
    const global = await readIfExists(path.join(this.globalDir, "AGENTS.md"));
    const project = await readIfExists(path.join(this.projectDir, "AGENTS.md"));

    const sections: string[] = [];
    if (global && global.trim().length > 0)
      sections.push(`## Global Instructions\n\n${global.trim()}`);
    if (project && project.trim().length > 0)
      sections.push(`## Project Instructions\n\n${project.trim()}`);
    return sections.join("\n\n---\n\n");
  }
}
