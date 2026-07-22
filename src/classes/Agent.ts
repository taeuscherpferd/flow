import { OllamaProvider } from "../providers/OllamaProvider.js";
import { AgentComsService } from "../services/AgentComsService.js";
import { ConfigService } from "../services/ConfigService.js";
import type { SkillFrontmatter } from "../services/SkillsService.js";
import { SkillsService } from "../services/SkillsService.js";
import { ToolRegistry } from "../tools/index.js";
import type { ToolExecutionContext } from "../tools/types.js";

function buildSystemPrompt(
  soul: string,
  agentsInstructions: string,
  skills: SkillFrontmatter[],
): string {
  const sections = [soul.trim()];

  if (agentsInstructions.trim().length > 0) {
    sections.push(agentsInstructions.trim());
  }

  if (skills.length > 0) {
    const listing = skills.map((s) => `- **${s.name}**: ${s.description}`).join("\n");
    sections.push(
      `## Available Skills\n\nCall the "load_skill" tool with a skill's name to load its full instructions when relevant to the current task.\n\n${listing}`,
    );
  }

  sections.push(
    "## Tools\n\nYou have access to read_file, write_file, run_command, and load_skill. Use them when they help complete the user's request.",
  );

  return sections.join("\n\n---\n\n");
}

export class Agent {
  private constructor(
    private readonly skillsService: SkillsService,
    private readonly agentComs: AgentComsService,
  ) {}

  static async create(): Promise<Agent> {
    const configService = new ConfigService();
    const config = await configService.load();

    const skillsService = new SkillsService(config.globalDir, config.projectDir);
    await skillsService.load();

    const providerConfig = config.models.providers[config.models.defaultProvider];
    if (!providerConfig) {
      throw new Error(`No provider config found for "${config.models.defaultProvider}".`);
    }
    const modelEntry = providerConfig.models.find((m) => m.name === config.models.defaultModel);
    if (!modelEntry) {
      throw new Error(`No model entry found for "${config.models.defaultModel}".`);
    }

    const provider = new OllamaProvider(providerConfig.baseUrl);
    const toolRegistry = new ToolRegistry(skillsService);
    const systemPrompt = buildSystemPrompt(config.soul, config.agentsInstructions, skillsService.listSkills());

    const toolCtx: ToolExecutionContext = {
      cwd: process.cwd(),
      requestPermission: async () => true,
    };

    const agentComs = new AgentComsService(
      provider,
      config.models.defaultModel,
      modelEntry.contextWindow,
      toolRegistry,
      systemPrompt,
      toolCtx,
    );

    return new Agent(skillsService, agentComs);
  }

  async handleUserMessage(text: string): Promise<string> {
    return this.agentComs.handleUserMessage(text);
  }

  /** Loads a skill's full body into context immediately, bypassing the model's own load_skill judgment. Returns false if no such skill exists. */
  loadSkillByName(name: string): boolean {
    const body = this.skillsService.getBody(name);
    if (body === undefined) return false;
    this.agentComs.injectSkillBody(name, body);
    return true;
  }

  listSkillNames(): string[] {
    return this.skillsService.listSkills().map((s) => s.name);
  }

  /** Clears the conversation history back to the initial system prompt. */
  clearHistory(): void {
    this.agentComs.clearHistory();
  }
}
