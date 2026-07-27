import {
  AGENT_NAME_PATTERN,
  AgentToolName,
  type AgentDefinition,
  type AgentPackageFingerprint,
  type AgentRecord,
} from "#src/agents/types.js";
import { ThinkingMode } from "#src/providers/types.js";
import type { ModelsConfig } from "#src/services/ConfigService.js";
import {
  fingerprintDirectory,
  listRegularFiles,
} from "#src/services/DirectoryFingerprint.js";
import { SkillsService } from "#src/services/SkillsService.js";
import { prepareWorkflowEsmScope } from "#src/workflows/WorkflowRegistry.js";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { parse as parseYaml } from "yaml";

AgentToolName;
const TOOL_NAMES: AgentToolName[] = Object.values(AgentToolName);
const THINKING_MODES: ThinkingMode[] = Object.values(ThinkingMode);

interface ManifestData {
  version?: number;
  name?: string;
  description?: string;
  model?: string;
  thinking?: string;
  tools?: string[];
}

export async function fingerprintAgentPackage(
  directory: string,
): Promise<AgentPackageFingerprint> {
  return {
    algorithm: "sha256",
    value: await fingerprintDirectory(directory),
  };
}

function resolvesModel(models: ModelsConfig, requested: string): boolean {
  const aliased = models.modelAliases?.[requested] ?? requested;
  const slash = aliased.indexOf("/");
  if (slash >= 1) {
    const provider = aliased.slice(0, slash);
    const model = aliased.slice(slash + 1);
    return (
      models.providers[provider]?.models.some(
        (entry) => entry.name === model,
      ) ?? false
    );
  }
  return (
    Object.values(models.providers).flatMap((provider) =>
      provider.models.filter((entry) => entry.name === aliased),
    ).length === 1
  );
}

function validateManifest(
  value: ManifestData,
  directoryName: string,
  models: ModelsConfig,
): AgentDefinition {
  if (value.version !== 1) {
    throw new Error('"version" must be 1');
  }
  if (value.name !== directoryName || !AGENT_NAME_PATTERN.test(directoryName)) {
    throw new Error(
      `"name" must match directory "${directoryName}" and use lowercase kebab-case`,
    );
  }
  if (
    typeof value.description !== "string" ||
    value.description.trim().length === 0
  ) {
    throw new Error('"description" must be a non-empty string');
  }
  if (value.model !== undefined && !resolvesModel(models, value.model)) {
    throw new Error(
      `"model" resolves to unknown or ambiguous model "${value.model}"`,
    );
  }
  if (
    value.thinking !== undefined &&
    !THINKING_MODES.includes(value.thinking as ThinkingMode)
  ) {
    throw new Error(`"thinking" is not a supported thinking mode`);
  }
  const tools = value.tools ?? ["read_file", "load_skill"];
  if (
    !Array.isArray(tools) ||
    tools.some((tool) => !TOOL_NAMES.includes(tool as AgentToolName))
  ) {
    throw new Error(
      `"tools" may contain only supported tool names: ${TOOL_NAMES.join(", ")}`,
    );
  }
  return {
    version: 1,
    name: value.name,
    description: value.description.trim(),
    ...(value.model === undefined ? {} : { model: value.model }),
    ...(value.thinking === undefined
      ? {}
      : { thinking: value.thinking as ThinkingMode }),
    tools: [...new Set(tools)] as AgentToolName[],
  };
}

async function readRequired(directory: string, name: string): Promise<string> {
  try {
    return await readFile(path.join(directory, name), "utf-8");
  } catch {
    throw new Error(`required file ${name} is missing`);
  }
}

async function readOptional(
  directory: string,
  name: string,
): Promise<string | undefined> {
  try {
    return await readFile(path.join(directory, name), "utf-8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
}

async function listContextFiles(directory: string): Promise<string[]> {
  const contextDir = path.join(directory, "context");
  try {
    return (await listRegularFiles(contextDir))
      .sort((left, right) => left.localeCompare(right))
      .map((file) => `context/${file.replaceAll(path.sep, "/")}`);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
}

export interface AgentPackageRegistryOptions {
  globalDir: string;
  projectDir: string;
  models: ModelsConfig;
  skillsConfig?: ConstructorParameters<typeof SkillsService>[2];
}

export class AgentPackageRegistry {
  private readonly agents = new Map<string, AgentRecord>();

  constructor(
    private readonly options: AgentPackageRegistryOptions,
    private readonly warn: (message: string) => void = (message) =>
      console.warn(message),
  ) {}

  async load(): Promise<void> {
    this.agents.clear();
    await this.scan(path.join(this.options.globalDir, "agents"), "global");
    await this.scan(path.join(this.options.projectDir, "agents"), "project");
  }

  list(): AgentRecord[] {
    return Array.from(this.agents.values()).sort((left, right) =>
      left.definition.name.localeCompare(right.definition.name),
    );
  }

  get(name: string): AgentRecord | undefined {
    return this.agents.get(name);
  }

  private async scan(
    agentsDir: string,
    source: "global" | "project",
  ): Promise<void> {
    let entries;
    try {
      entries = await readdir(agentsDir, { withFileTypes: true });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
      throw error;
    }
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      if (source === "project") this.agents.delete(entry.name);
      await this.loadDirectory(
        path.join(agentsDir, entry.name),
        entry.name,
        source,
      );
    }
  }

  private async loadDirectory(
    directory: string,
    name: string,
    source: "global" | "project",
  ): Promise<void> {
    try {
      if (!AGENT_NAME_PATTERN.test(name)) {
        throw new Error("directory names must use lowercase kebab-case");
      }
      const rawManifest = await readRequired(directory, "AGENT.yaml");
      const parsed = parseYaml(rawManifest) as ManifestData;
      const definition = validateManifest(parsed, name, this.options.models);
      const soul = await readRequired(directory, "SOUL.md");
      const instructions = await readRequired(directory, "AGENTS.md");
      const contextIndex = await readOptional(directory, "CONTEXT.md");
      await fingerprintAgentPackage(directory);
      await prepareWorkflowEsmScope(directory);
      const skills = new SkillsService(
        directory,
        directory,
        this.options.skillsConfig,
        [{ directory: path.join(directory, "skills"), source }],
      );
      await skills.load();
      const skillRecords = skills.listRecords(name);
      for (const skill of skillRecords) {
        if (
          skill.frontmatter.name !== path.basename(skill.dir) ||
          !AGENT_NAME_PATTERN.test(skill.frontmatter.name)
        ) {
          throw new Error(
            `skill "${skill.frontmatter.name}" must match its lowercase kebab-case directory`,
          );
        }
      }
      this.agents.set(name, {
        definition,
        directory,
        source,
        soul,
        instructions,
        ...(contextIndex === undefined ? {} : { contextIndex }),
        contextFiles: await listContextFiles(directory),
        skills: skillRecords,
        fingerprint: await fingerprintAgentPackage(directory),
      });
    } catch (error) {
      this.warn(
        `Skipping agent "${name}" — ${
          error instanceof Error ? error.message : String(error)
        }.`,
      );
    }
  }
}
