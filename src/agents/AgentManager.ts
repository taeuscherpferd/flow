import { AgentConversationStore } from "#src/agents/AgentConversationStore.js";
import { AgentPackageRegistry } from "#src/agents/AgentPackageRegistry.js";
import { AgentSkillCatalog } from "#src/agents/AgentSkillCatalog.js";
import {
  createAgentWorkflowRegistries,
  type AgentExecutionScope,
} from "#src/agents/AgentWorkflowRegistryCatalog.js";
import {
  AgentToolName,
  type AgentExecutionMode,
  type AgentProfile,
  type AgentRecord,
  type AgentSessionRecord,
} from "#src/agents/types.js";
import { Agent } from "#src/classes/Agent.js";
import type { ChatMessage } from "#src/providers/types.js";
import {
  ConfigService,
  type ResolvedConfig,
} from "#src/services/ConfigService.js";
import { SkillsService } from "#src/services/SkillsService.js";
import {
  createDelegateAgentTool,
  createListAgentsTool,
} from "#src/tools/agentCoordination.js";
import type { Tool, ToolEffect } from "#src/tools/types.js";
import type { JsonObject } from "#src/workflows/types.js";
import { WorkflowRegistry } from "#src/workflows/WorkflowRegistry.js";
import { randomUUID } from "node:crypto";
import path from "node:path";

export interface ListedAgent {
  name: string;
  description: string;
  source: "built-in" | "global" | "project";
  active: boolean;
}

export interface AgentManagerOptions {
  requestPermission?: (
    toolName: string,
    args: JsonObject,
    effect?: ToolEffect,
  ) => Promise<boolean>;
}

interface ManagedRuntime {
  agent: Agent;
  session: AgentSessionRecord;
}

export class AgentManager {
  private readonly runtimes = new Map<string, ManagedRuntime>();
  private readonly workflowRegistries = new Map<string, WorkflowRegistry>();
  private activeAgentName = "main";
  private readonly directTools: Tool[] = [];

  private constructor(
    private readonly config: ResolvedConfig,
    private readonly packageRegistry: AgentPackageRegistry,
    private readonly rootSkills: SkillsService,
    private readonly conversationStore: AgentConversationStore | undefined,
    private readonly options: AgentManagerOptions,
  ) {}

  static async create(
    configService: ConfigService = new ConfigService(),
    options: AgentManagerOptions = {},
  ): Promise<AgentManager> {
    return AgentManager.initialize(configService, options, true);
  }

  static async createExecution(
    configService: ConfigService,
    options: AgentManagerOptions = {},
    scope?: AgentExecutionScope,
  ): Promise<AgentManager> {
    return AgentManager.initialize(configService, options, false, scope);
  }

  private static async initialize(
    configService: ConfigService,
    options: AgentManagerOptions,
    initializeDirectConversation: boolean,
    executionScope?: AgentExecutionScope,
  ): Promise<AgentManager> {
    const config = await configService.load();
    configService.validateModelsConfig(config.models);
    const rootSkills = new SkillsService(
      config.globalDir,
      config.projectDir,
      config.skillsConfig,
    );
    await rootSkills.load();
    const packageRegistry = new AgentPackageRegistry({
      globalDir: config.globalDir,
      projectDir: config.projectDir,
      models: config.models,
      skillsConfig: config.skillsConfig,
    });
    await packageRegistry.load();
    const manager = new AgentManager(
      config,
      packageRegistry,
      rootSkills,
      initializeDirectConversation
        ? new AgentConversationStore(config.globalDir)
        : undefined,
      options,
    );
    const workflowRegistries = await createAgentWorkflowRegistries(
      config,
      packageRegistry,
      executionScope,
    );
    for (const [name, registry] of workflowRegistries) {
      manager.workflowRegistries.set(name, registry);
    }
    if (initializeDirectConversation) {
      await manager.getOrCreateRuntime("main", "direct");
    }
    return manager;
  }

  get projectDir(): string {
    return path.dirname(this.config.projectDir);
  }

  get globalDir(): string {
    return this.config.globalDir;
  }

  getActiveName(): string {
    return this.activeAgentName;
  }

  getActiveAgent(): Agent {
    const runtime = this.runtimes.get(this.activeAgentName);
    if (!runtime) {
      throw new Error("Direct agent conversations are unavailable.");
    }
    return runtime.agent;
  }

  getActiveWorkflowRegistry(): WorkflowRegistry {
    return this.workflowRegistries.get(this.activeAgentName)!;
  }

  getWorkflowRegistry(name: string): WorkflowRegistry | undefined {
    return this.workflowRegistries.get(name);
  }

  getPackage(name: string): AgentRecord | undefined {
    return this.packageRegistry.get(name);
  }

  listAgents(): ListedAgent[] {
    return [
      {
        name: "main",
        description: "Coordinates work across configured specialists",
        source: "built-in",
        active: this.activeAgentName === "main",
      },
      ...this.packageRegistry.list().map((record) => ({
        name: record.definition.name,
        description: record.definition.description,
        source: record.source,
        active: this.activeAgentName === record.definition.name,
      })),
    ];
  }

  async switchAgent(name: string): Promise<Agent> {
    if (name !== "main" && !this.packageRegistry.get(name)) {
      throw new Error(`Unknown agent "${name}".`);
    }
    const runtime = await this.getOrCreateRuntime(name, "direct");
    this.activeAgentName = name;
    return runtime.agent;
  }

  async delegate(
    name: string,
    task: string,
    signal?: AbortSignal,
  ): Promise<string> {
    if (name === "main") {
      throw new Error("The coordinator cannot delegate recursively to itself.");
    }
    const record = this.packageRegistry.get(name);
    if (!record) throw new Error(`Unknown agent "${name}".`);
    signal?.throwIfAborted();
    const agent = this.createRuntimeAgent(
      name,
      "delegated",
      undefined,
      undefined,
      true,
    );
    return agent.handleUserMessage(task, signal);
  }

  createExecutionAgent(
    name: string,
    mode: Exclude<AgentExecutionMode, "direct">,
  ): Agent {
    return this.createRuntimeAgent(name, mode, undefined, undefined, true);
  }

  registerDirectTool(tool: Tool): void {
    this.directTools.push(tool);
    for (const runtime of this.runtimes.values()) {
      runtime.agent.registerDirectTool(tool);
    }
  }

  clearActiveHistory(): void {
    const runtime = this.runtimes.get(this.activeAgentName);
    if (!runtime || !this.conversationStore) {
      throw new Error("Direct agent conversations are unavailable.");
    }
    runtime.agent.clearHistory();
    this.conversationStore.clear(this.projectDir, this.activeAgentName);
    this.persistRuntime(this.activeAgentName);
  }

  persistActive(): void {
    this.persistRuntime(this.activeAgentName);
  }

  close(): void {
    if (!this.conversationStore) return;
    for (const name of this.runtimes.keys()) this.persistRuntime(name);
    this.conversationStore.close();
  }

  private async getOrCreateRuntime(
    name: string,
    mode: AgentExecutionMode,
  ): Promise<ManagedRuntime> {
    const existing = this.runtimes.get(name);
    if (existing) return existing;
    const stored = this.conversationStore?.get(this.projectDir, name);
    let agent: Agent;
    try {
      agent = this.createRuntimeAgent(
        name,
        mode,
        stored?.history,
        stored
          ? `${stored.session.provider}/${stored.session.model}`
          : undefined,
        false,
      );
    } catch (error) {
      if (!stored) throw error;
      agent = this.createRuntimeAgent(
        name,
        mode,
        stored.history,
        undefined,
        false,
      );
    }
    const model = agent.getCurrentModel();
    const now = new Date().toISOString();
    const session: AgentSessionRecord = stored?.session ?? {
      id: randomUUID(),
      projectDir: this.projectDir,
      agentName: name,
      mode,
      provider: model.provider,
      model: model.model,
      createdAt: now,
      updatedAt: now,
    };
    const runtime = { agent, session };
    this.runtimes.set(name, runtime);
    if (name === "main") {
      agent.registerDirectTool(createListAgentsTool(this));
      agent.registerDirectTool(createDelegateAgentTool(this));
    }
    for (const tool of this.directTools) agent.registerDirectTool(tool);
    return runtime;
  }

  private createRuntimeAgent(
    name: string,
    mode: AgentExecutionMode,
    history?: ChatMessage[],
    modelSpec?: string,
    delegated = false,
  ): Agent {
    const records = this.packageRegistry.list();
    const record = name === "main" ? undefined : this.packageRegistry.get(name);
    if (name !== "main" && !record) throw new Error(`Unknown agent "${name}".`);
    const catalog =
      name === "main"
        ? new AgentSkillCatalog(
            this.rootSkills.listRecords(),
            records.map((entry) => ({
              agentName: entry.definition.name,
              skills: entry.skills,
            })),
            "main",
          )
        : new AgentSkillCatalog(
            [],
            [{ agentName: name, skills: record!.skills }],
            name,
          );
    const profile = this.createProfile(name, record, delegated);
    return Agent.createProfile({
      config: this.config,
      profile,
      skills: catalog,
      mode,
      ...(history === undefined ? {} : { history }),
      ...(modelSpec === undefined ? {} : { modelSpec }),
      agents:
        name === "main" && mode === "direct"
          ? records.map((entry) => ({
              name: entry.definition.name,
              description: entry.definition.description,
            }))
          : [],
      requestPermission: this.options.requestPermission ?? (async () => false),
      ...(this.conversationStore === undefined
        ? {}
        : { onHistoryChange: () => this.persistRuntime(name) }),
    });
  }

  private createProfile(
    name: string,
    record: AgentRecord | undefined,
    delegated: boolean,
  ): AgentProfile {
    if (!record) {
      return {
        name: "main",
        description: "Coordinates work across configured specialists",
        soul: this.config.soul,
        instructions: this.config.agentsInstructions,
        contextFiles: [],
        tools: delegated
          ? [AgentToolName.ReadFile, AgentToolName.WriteFile, AgentToolName.RunCommand, AgentToolName.LoadSkill]
          : [
              AgentToolName.ReadFile,
              AgentToolName.WriteFile,
              AgentToolName.RunCommand,
              AgentToolName.LoadSkill,
              AgentToolName.RunWorkflow,
              AgentToolName.ListAgents,
              AgentToolName.DelegateAgent,
              AgentToolName.CreateSchedule,
              AgentToolName.ListSchedules,
              AgentToolName.PauseSchedule,
              AgentToolName.ResumeSchedule,
              AgentToolName.DeleteSchedule,
            ],
      };
    }
    const specialistTools = record.definition.tools.filter(
      (tool) => tool !== AgentToolName.DelegateAgent && tool !== AgentToolName.ListAgents,
    );
    return {
      name,
      description: record.definition.description,
      soul: record.soul,
      instructions: record.instructions,
      ...(record.contextIndex === undefined
        ? {}
        : { contextIndex: record.contextIndex }),
      contextFiles: record.contextFiles,
      packageDirectory: record.directory,
      ...(record.definition.model === undefined
        ? {}
        : { model: record.definition.model }),
      ...(record.definition.thinking === undefined
        ? {}
        : { thinking: record.definition.thinking }),
      tools: delegated
        ? specialistTools.filter((tool) => !tool.includes("schedule"))
        : [
            ...new Set([
              ...specialistTools,
              AgentToolName.CreateSchedule,
              AgentToolName.ListSchedules,
              AgentToolName.PauseSchedule,
              AgentToolName.ResumeSchedule,
              AgentToolName.DeleteSchedule,
            ]),
          ],
      fingerprint: record.fingerprint,
    };
  }

  private persistRuntime(name: string): void {
    if (!this.conversationStore) return;
    const runtime = this.runtimes.get(name);
    if (!runtime) return;
    const model = runtime.agent.getCurrentModel();
    runtime.session.provider = model.provider;
    runtime.session.model = model.model;
    runtime.session.updatedAt = new Date().toISOString();
    this.conversationStore.save(
      runtime.session,
      runtime.agent.snapshotHistory(),
    );
  }
}
