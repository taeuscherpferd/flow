import { randomUUID } from "node:crypto";
import path from "node:path";
import { OllamaProvider } from "#src/providers/OllamaProvider.js";
import type { ChatMessage, ModelProvider } from "#src/providers/types.js";
import { AgentComsService } from "#src/services/AgentComsService.js";
import type { ModelsConfig, ProviderConfig } from "#src/services/ConfigService.js";
import { ConfigService } from "#src/services/ConfigService.js";
import { EnvSecretsProvider } from "#src/services/SecretsProvider.js";
import type { SkillFrontmatter } from "#src/services/SkillsService.js";
import { SkillsService } from "#src/services/SkillsService.js";
import {
  buildWorkflowSystemContext,
  createRunWorkflowTool,
  type WorkflowToolRuntime,
} from "#src/tools/runWorkflow.js";
import { ToolRegistry } from "#src/tools/ToolRegistry.js";
import type { ToolExecutionContext } from "#src/tools/types.js";
import type { JsonValue, WorkflowRecord } from "#src/workflows/types.js";
import { AgentSession } from "#src/classes/AgentSession.js";

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
  private workflowSystemContext = "";

  private constructor(
    private readonly skillsService: SkillsService,
    private readonly agentComs: AgentComsService,
    private readonly models: ModelsConfig,
    private readonly providers: Map<string, ModelProvider>,
    private readonly baseToolRegistry: ToolRegistry,
    private readonly mainToolRegistry: ToolRegistry,
    private readonly systemPrompt: string,
    private readonly toolCtx: ToolExecutionContext,
    initialProvider: string,
  ) {
    this.currentProvider = initialProvider;
  }

  static async create(
    configService: ConfigService = new ConfigService(),
  ): Promise<Agent> {
    const config = await configService.load();
    configService.validateModelsConfig(config.models);

    // Project `.env` is listed first so its values win over the global one;
    // real shell environment variables win over both 
    const secrets = new EnvSecretsProvider([
      path.join(config.projectDir, ".env"),
      path.join(config.globalDir, ".env"),
    ]);

    const skillsService = new SkillsService(config.globalDir, config.projectDir, config.skillsConfig);
    await skillsService.load();
    skillsService.validateSecrets(secrets);

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

    const baseToolRegistry = new ToolRegistry(skillsService);
    const mainToolRegistry = new ToolRegistry(skillsService);
    const systemPrompt = buildSystemPrompt(config.soul, config.agentsInstructions, skillsService.listSkills());

    const toolCtx: ToolExecutionContext = {
      cwd: process.cwd(),
      requestPermission: async () => true,
      secrets,
    };

    const agentComs = new AgentComsService(
      providers.get(config.models.defaultProvider)!,
      config.models.defaultModel,
      modelEntry.contextWindow,
      mainToolRegistry,
      systemPrompt,
      toolCtx,
    );

    return new Agent(
      skillsService,
      agentComs,
      config.models,
      providers,
      baseToolRegistry,
      mainToolRegistry,
      systemPrompt,
      toolCtx,
      config.models.defaultProvider,
    );
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

  getCurrentModel(): { provider: string; model: string } {
    return { provider: this.currentProvider, model: this.agentComs.getModel() };
  }

  setModel(
    spec: string,
  ): { ok: true; changed: boolean } | { ok: false; error: string } {
    const resolved = this.resolveModel(spec);
    if (!resolved.ok) return resolved;
    const { providerName, modelName, contextWindow } = resolved;
    const changed =
      this.currentProvider !== providerName ||
      this.agentComs.getModel() !== modelName;
    this.agentComs.setModel(this.providers.get(providerName)!, modelName, contextWindow);
    this.currentProvider = providerName;
    return { ok: true, changed };
  }

  createSession(
    modelSpec: string,
    history?: ChatMessage[],
    sessionId: string = randomUUID(),
    workflowAccess: "disabled" | "eligible" = "disabled",
  ): AgentSession {
    const resolved = this.resolveModel(modelSpec);
    if (!resolved.ok) throw new Error(resolved.error);

    const agentComs = new AgentComsService(
      this.providers.get(resolved.providerName)!,
      resolved.modelName,
      resolved.contextWindow,
      workflowAccess === "eligible"
        ? this.mainToolRegistry
        : this.baseToolRegistry,
      history ?? this.systemPrompt,
      this.toolCtx,
    );
    if (
      workflowAccess === "eligible" &&
      this.workflowSystemContext.length > 0 &&
      !agentComs
        .snapshotHistory()
        .some(
          (message) =>
            message.role === "system" &&
            message.content === this.workflowSystemContext,
        )
    ) {
      agentComs.injectSystemContext(this.workflowSystemContext);
    }

    return new AgentSession(
      sessionId,
      resolved.providerName,
      agentComs,
    );
  }

  forkSession(
    session: AgentSession,
    modelSpec?: string,
    workflowAccess: "disabled" | "eligible" = "disabled",
  ): AgentSession {
    const current = session.getModel();
    return this.createSession(
      modelSpec ?? `${current.provider}/${current.model}`,
      session.snapshotHistory(),
      randomUUID(),
      workflowAccess,
    );
  }

  configureWorkflows(
    workflows: WorkflowRecord[],
    runtime: WorkflowToolRuntime,
  ): void {
    const context = buildWorkflowSystemContext(workflows);
    if (context.length === 0) return;
    this.workflowSystemContext = context;
    this.mainToolRegistry.register(createRunWorkflowTool(workflows, runtime));
    this.agentComs.injectSystemContext(context);
  }

  retargetSession(session: AgentSession, modelSpec: string): void {
    const resolved = this.resolveModel(modelSpec);
    if (!resolved.ok) throw new Error(resolved.error);
    session.retarget(
      resolved.providerName,
      this.providers.get(resolved.providerName)!,
      resolved.modelName,
      resolved.contextWindow,
    );
  }

  async handleUserMessage(text: string): Promise<string> {
    return this.agentComs.handleUserMessage(text);
  }

  async presentWorkflowResult(
    workflowName: string,
    value: JsonValue,
  ): Promise<string> {
    const current = this.getCurrentModel();
    const presentationSession = this.createSession(
      `${current.provider}/${current.model}`,
    );
    return presentationSession.run(
      `Workflow "${workflowName}" completed with this durable JSON result:\n\n` +
        `${JSON.stringify(value, null, 2)}\n\n` +
        "Present the result clearly to the user. Do not run another workflow.",
      { tools: "none" },
    );
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
    this.agentComs.clearHistory(
      this.workflowSystemContext.length === 0
        ? []
        : [this.workflowSystemContext],
    );
  }

  private resolveModel(
    requestedSpec: string,
  ):
    | {
        ok: true;
        providerName: string;
        modelName: string;
        contextWindow: number;
      }
    | { ok: false; error: string } {
    const aliasTarget = this.models.modelAliases?.[requestedSpec];
    const spec = aliasTarget ?? requestedSpec;
    const slash = spec.indexOf("/");

    let providerName: string;
    let modelName: string;
    if (slash !== -1) {
      providerName = spec.slice(0, slash).trim();
      modelName = spec.slice(slash + 1).trim();
      const config = this.models.providers[providerName];
      if (!config) {
        return { ok: false, error: `Unknown provider "${providerName}".` };
      }
      if (!config.models.some((model) => model.name === modelName)) {
        return {
          ok: false,
          error: `Provider "${providerName}" has no model "${modelName}".`,
        };
      }
    } else {
      const matches = this.listModels().filter(
        (reference) => reference.model === spec,
      );
      if (matches.length === 0) {
        return { ok: false, error: `Unknown model "${requestedSpec}".` };
      }
      if (matches.length > 1) {
        const qualified = matches
          .map((match) => `${match.provider}/${match.model}`)
          .join(", ");
        return {
          ok: false,
          error: `Model "${requestedSpec}" exists in multiple providers — qualify it: ${qualified}.`,
        };
      }
      providerName = matches[0]!.provider;
      modelName = matches[0]!.model;
    }

    const contextWindow = this.models.providers[
      providerName
    ]!.models.find((model) => model.name === modelName)!.contextWindow;
    return {
      ok: true,
      providerName,
      modelName,
      contextWindow,
    };
  }
}
