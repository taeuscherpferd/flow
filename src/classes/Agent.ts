import { OllamaProvider } from "../providers/OllamaProvider.js";
import type { ModelProvider } from "../providers/types.js";
import { AgentComsService } from "../services/AgentComsService.js";
import type { ModelsConfig, ProviderConfig } from "../services/ConfigService.js";
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

/**
 * Instantiates a provider from its config. All configured backends currently
 * speak the Ollama chat protocol (distinguished only by base URL); add cases
 * here when a genuinely different provider type is introduced.
 */
function createProvider(_name: string, config: ProviderConfig): ModelProvider {
  return new OllamaProvider(config.baseUrl);
}

/** A fully-qualified reference to a model within a specific provider. */
export interface ModelRef {
  provider: string;
  model: string;
  active: boolean;
}

export class Agent {
  private currentProvider: string;

  private constructor(
    private readonly skillsService: SkillsService,
    private readonly agentComs: AgentComsService,
    private readonly models: ModelsConfig,
    private readonly providers: Map<string, ModelProvider>,
    initialProvider: string,
  ) {
    this.currentProvider = initialProvider;
  }

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

    // Instantiate every configured provider once, so /model can swap between them.
    const providers = new Map<string, ModelProvider>();
    for (const [name, cfg] of Object.entries(config.models.providers)) {
      providers.set(name, createProvider(name, cfg));
    }

    const toolRegistry = new ToolRegistry(skillsService);
    const systemPrompt = buildSystemPrompt(config.soul, config.agentsInstructions, skillsService.listSkills());

    const toolCtx: ToolExecutionContext = {
      cwd: process.cwd(),
      requestPermission: async () => true,
    };

    const agentComs = new AgentComsService(
      providers.get(config.models.defaultProvider)!,
      config.models.defaultModel,
      modelEntry.contextWindow,
      toolRegistry,
      systemPrompt,
      toolCtx,
    );

    return new Agent(skillsService, agentComs, config.models, providers, config.models.defaultProvider);
  }

  /** Lists every configured model across all providers, flagging the active one. */
  listModels(): ModelRef[] {
    const activeModel = this.agentComs.getModel();
    const refs: ModelRef[] = [];
    for (const [provider, cfg] of Object.entries(this.models.providers)) {
      for (const m of cfg.models) {
        refs.push({
          provider,
          model: m.name,
          active: provider === this.currentProvider && m.name === activeModel,
        });
      }
    }
    return refs;
  }

  /** Returns the provider and model currently in use. */
  getCurrentModel(): { provider: string; model: string } {
    return { provider: this.currentProvider, model: this.agentComs.getModel() };
  }

  /**
   * Switches the active model. `spec` may be provider-qualified ("ollama/llama3.1")
   * or a bare model name, which is accepted only if it's unique across providers.
   * History is preserved across the swap. Returns { ok: true } on success, or
   * { ok: false, error } describing why the swap was rejected.
   */
  setModel(spec: string): { ok: true } | { ok: false; error: string } {
    const slash = spec.indexOf("/");

    let providerName: string;
    let modelName: string;
    if (slash !== -1) {
      providerName = spec.slice(0, slash).trim();
      modelName = spec.slice(slash + 1).trim();
      const cfg = this.models.providers[providerName];
      if (!cfg) return { ok: false, error: `Unknown provider "${providerName}".` };
      if (!cfg.models.some((m) => m.name === modelName)) {
        return { ok: false, error: `Provider "${providerName}" has no model "${modelName}".` };
      }
    } else {
      // Bare model name: resolve against all providers, requiring a unique match.
      const matches = this.listModels().filter((r) => r.model === spec);
      if (matches.length === 0) return { ok: false, error: `Unknown model "${spec}".` };
      if (matches.length > 1) {
        const qualified = matches.map((m) => `${m.provider}/${m.model}`).join(", ");
        return {
          ok: false,
          error: `Model "${spec}" exists in multiple providers — qualify it: ${qualified}.`,
        };
      }
      providerName = matches[0]!.provider;
      modelName = matches[0]!.model;
    }

    const contextWindow = this.models.providers[providerName]!.models.find(
      (m) => m.name === modelName,
    )!.contextWindow;
    this.agentComs.setModel(this.providers.get(providerName)!, modelName, contextWindow);
    this.currentProvider = providerName;
    return { ok: true };
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
