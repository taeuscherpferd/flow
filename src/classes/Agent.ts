import {
  createProviders,
  listModelReferences,
  resolveModel,
} from "#src/agents/AgentModelRuntime.js";
import {
  buildSystemPrompt,
  type AgentDirectoryListing,
} from "#src/agents/AgentPrompt.js";
import {
  AgentToolName,
  type AgentExecutionMode,
  type AgentProfile,
} from "#src/agents/types.js";
import { AgentSession } from "#src/classes/AgentSession.js";
import type {
  ChatMessage,
  ModelProvider,
  ThinkingMode,
} from "#src/providers/types.js";
import { AgentComsService } from "#src/services/AgentComsService.js";
import type { ModelsConfig, ResolvedConfig } from "#src/services/ConfigService.js";
import { ConfigService } from "#src/services/ConfigService.js";
import { EnvSecretsProvider } from "#src/services/SecretsProvider.js";
import { SkillsService } from "#src/services/SkillsService.js";
import type { SkillLoader } from "#src/tools/loadSkill.js";
import {
  buildWorkflowSystemContext,
  createRunWorkflowTool,
  type WorkflowToolRuntime,
} from "#src/tools/runWorkflow.js";
import { ToolRegistry } from "#src/tools/ToolRegistry.js";
import type {
  Tool,
  ToolEffect,
  ToolExecutionContext,
} from "#src/tools/types.js";
import type { JsonObject, JsonValue, WorkflowRecord } from "#src/workflows/types.js";
import { randomUUID } from "node:crypto";
import path from "node:path";

export interface ModelRef {
  provider: string;
  model: string;
  active: boolean;
}

export interface CreateProfileAgentOptions {
  config: ResolvedConfig;
  profile: AgentProfile;
  skills: SkillLoader;
  mode?: AgentExecutionMode;
  history?: ChatMessage[];
  modelSpec?: string;
  agents?: AgentDirectoryListing[];
  requestPermission?: (
    toolName: string,
    args: JsonObject,
    effect?: ToolEffect,
  ) => Promise<boolean>;
  onHistoryChange?(history: ChatMessage[]): void;
}

export class Agent {
  private currentProvider: string;
  private workflowSystemContext = "";

  private constructor(
    private readonly profile: AgentProfile,
    private readonly skillsService: SkillLoader,
    private readonly agentComs: AgentComsService,
    private readonly models: ModelsConfig,
    private readonly providers: Map<string, ModelProvider>,
    private readonly baseToolRegistry: ToolRegistry,
    private readonly mainToolRegistry: ToolRegistry,
    private readonly systemPrompt: string,
    private readonly toolCtx: ToolExecutionContext,
    private readonly executionMode: AgentExecutionMode,
    initialProvider: string,
  ) {
    this.currentProvider = initialProvider;
  }

  static async create(
    configService: ConfigService = new ConfigService(),
  ): Promise<Agent> {
    const config = await configService.load();
    configService.validateModelsConfig(config.models);
    const skills = new SkillsService(
      config.globalDir,
      config.projectDir,
      config.skillsConfig,
    );
    await skills.load();
    const secrets = new EnvSecretsProvider([
      path.join(config.projectDir, ".env"),
      path.join(config.globalDir, ".env"),
    ]);
    skills.validateSecrets(secrets);
    return Agent.createProfile({
      config,
      skills,
      profile: {
        name: "main",
        description: "Coordinates work across the current project",
        soul: config.soul,
        instructions: config.agentsInstructions,
        contextFiles: [],
        tools: [
          AgentToolName.ReadFile,
          AgentToolName.WriteFile,
          AgentToolName.RunCommand,
          AgentToolName.LoadSkill,
          AgentToolName.RunWorkflow,
        ],
      },
    });
  }

  static createProfile(options: CreateProfileAgentOptions): Agent {
    const { config, profile, skills } = options;
    const providers = createProviders(config.models);
    const requestedModel =
      options.modelSpec ??
      profile.model ??
      `${config.models.defaultProvider}/${config.models.defaultModel}`;
    const resolved = resolveModel(config.models, requestedModel);
    if (!resolved.ok) throw new Error(resolved.error);

    const secrets = new EnvSecretsProvider([
      path.join(config.projectDir, ".env"),
      path.join(config.globalDir, ".env"),
    ]);
    const toolCtx: ToolExecutionContext = {
      cwd: path.dirname(config.projectDir),
      requestPermission: options.requestPermission ?? (async () => false),
      secrets,
      executionMode: options.mode ?? "direct",
    };
    const baseToolRegistry = new ToolRegistry(skills, profile.tools);
    const mainToolRegistry = new ToolRegistry(skills, profile.tools);
    const systemPrompt = buildSystemPrompt(
      profile,
      skills.listSkills(),
      options.agents,
    );
    const history = [
      { role: "system" as const, content: systemPrompt },
      ...(options.history ?? []).filter((message) => message.role !== "system"),
    ];
    const agentComs = new AgentComsService(
      providers.get(resolved.value.providerName)!,
      resolved.value.modelName,
      resolved.value.contextWindow,
      mainToolRegistry,
      history,
      toolCtx,
      options.onHistoryChange === undefined
        ? {}
        : { onHistoryChange: options.onHistoryChange },
    );
    return new Agent(
      profile,
      skills,
      agentComs,
      config.models,
      providers,
      baseToolRegistry,
      mainToolRegistry,
      systemPrompt,
      toolCtx,
      options.mode ?? "direct",
      resolved.value.providerName,
    );
  }

  getName(): string {
    return this.profile.name;
  }

  getThinking(): ThinkingMode | undefined {
    return this.profile.thinking;
  }

  getExecutionMode(): AgentExecutionMode {
    return this.executionMode;
  }

  listModels(): ModelRef[] {
    const activeModel = this.agentComs.getModel();
    return listModelReferences(this.models).map((reference) => ({
      ...reference,
      active:
        reference.provider === this.currentProvider &&
        reference.model === activeModel,
    }));
  }

  getCurrentModel(): { provider: string; model: string } {
    return { provider: this.currentProvider, model: this.agentComs.getModel() };
  }

  setModel(
    spec: string,
  ): { ok: true; changed: boolean } | { ok: false; error: string } {
    const resolved = resolveModel(this.models, spec);
    if (!resolved.ok) return resolved;
    const { providerName, modelName, contextWindow } = resolved.value;
    const changed =
      this.currentProvider !== providerName ||
      this.agentComs.getModel() !== modelName;
    this.agentComs.setModel(
      this.providers.get(providerName)!,
      modelName,
      contextWindow,
    );
    this.currentProvider = providerName;
    return { ok: true, changed };
  }

  createSession(
    modelSpec?: string,
    history?: ChatMessage[],
    sessionId: string = randomUUID(),
    workflowAccess: "disabled" | "eligible" = "disabled",
  ): AgentSession {
    const current = this.getCurrentModel();
    const resolved = resolveModel(
      this.models,
      modelSpec ?? `${current.provider}/${current.model}`,
    );
    if (!resolved.ok) throw new Error(resolved.error);
    const sessionMode: AgentExecutionMode =
      this.executionMode === "scheduled" ? "scheduled" : "workflow";
    const sessionContext: ToolExecutionContext = {
      ...this.toolCtx,
      executionMode: sessionMode,
    };
    const agentComs = new AgentComsService(
      this.providers.get(resolved.value.providerName)!,
      resolved.value.modelName,
      resolved.value.contextWindow,
      workflowAccess === "eligible"
        ? this.mainToolRegistry
        : this.baseToolRegistry,
      history ?? this.systemPrompt,
      sessionContext,
    );
    if (workflowAccess === "eligible" && this.workflowSystemContext.length > 0) {
      agentComs.injectSystemContext(this.workflowSystemContext);
    }
    return new AgentSession(
      sessionId,
      resolved.value.providerName,
      agentComs,
      this.profile.thinking,
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
    const previousContext = this.workflowSystemContext;
    this.workflowSystemContext = context;
    this.mainToolRegistry.register(createRunWorkflowTool(workflows, runtime));
    this.agentComs.replaceSystemContext(previousContext, context);
  }

  registerDirectTool(tool: Tool): void {
    this.mainToolRegistry.register(tool);
  }

  retargetSession(session: AgentSession, modelSpec: string): void {
    const resolved = resolveModel(this.models, modelSpec);
    if (!resolved.ok) throw new Error(resolved.error);
    session.retarget(
      resolved.value.providerName,
      this.providers.get(resolved.value.providerName)!,
      resolved.value.modelName,
      resolved.value.contextWindow,
    );
  }

  async handleUserMessage(
    text: string,
    signal?: AbortSignal,
  ): Promise<string> {
    return this.agentComs.handleUserMessage(
      text,
      this.profile.thinking === undefined
        ? {}
        : { thinking: this.profile.thinking },
      signal,
    );
  }

  async presentWorkflowResult(
    workflowName: string,
    value: JsonValue,
  ): Promise<string> {
    const current = this.getCurrentModel();
    const session = this.createSession(`${current.provider}/${current.model}`);
    return session.run(
      `Workflow "${workflowName}" completed with this durable JSON result:\n\n` +
        `${JSON.stringify(value, null, 2)}\n\n` +
        "Present the result clearly to the user. Do not run another workflow.",
      { tools: "none" },
    );
  }

  loadSkillByName(name: string): boolean {
    const body = this.skillsService.getBody(name);
    if (body === undefined) return false;
    this.agentComs.injectSkillBody(name, body);
    return true;
  }

  listSkillNames(): string[] {
    return this.skillsService.listSkills().map((skill) => skill.name);
  }

  clearHistory(): void {
    this.agentComs.clearHistory(
      this.workflowSystemContext.length === 0
        ? []
        : [this.workflowSystemContext],
    );
  }

  snapshotHistory(): ChatMessage[] {
    return this.agentComs.snapshotHistory();
  }

  rebuildSystemPrompt(): void {
    this.agentComs.replaceSystemPrompt(this.systemPrompt);
  }
}
