import { AgentManager } from "#src/agents/AgentManager.js";
import type { Agent } from "#src/classes/Agent.js";
import {
  WorkflowCliController,
  type WorkflowCliUi,
} from "#src/cli/WorkflowCliController.js";
import { ScheduleCliController } from "#src/cli/ScheduleCliController.js";
import { CliPermissionController } from "#src/cli/CliPermissionController.js";
import { PersistentInputHistory } from "#src/cli/PersistentInputHistory.js";
import {
  BUILTIN_COMMANDS,
  HELP_TEXT,
  READY_TEXT,
  WELCOME_TEXT,
} from "#src/cli/FlowCliHelp.js";
import { ConfigService } from "#src/services/ConfigService.js";
import { ModelSetupService } from "#src/services/ModelSetupService.js";
import { EOF, ghostPrompt } from "#src/ui/lineEditor.js";
import { startSpinner } from "#src/ui/spinner.js";
import type { WorkflowRegistry } from "#src/workflows/WorkflowRegistry.js";

export class FlowCli {
  private manager: AgentManager | undefined;
  private agent: Agent | undefined;
  private workflowRegistry: WorkflowRegistry | undefined;
  private workflowController: WorkflowCliController | undefined;
  private scheduleController: ScheduleCliController | undefined;
  private stopActiveSpinner: (() => void) | undefined;
  private inputHistory: PersistentInputHistory | undefined;
  private readonly permissionController: CliPermissionController;

  constructor(private readonly configService = new ConfigService()) {
    this.permissionController = new CliPermissionController({
      pauseSpinner: () => this.pauseSpinner(),
      resumeSpinner: () => this.startActiveSpinner(),
      getScheduleController: () => this.scheduleController,
    });
  }

  async run(): Promise<void> {
    if (!(await this.initialize())) return;
    console.log(this.agent ? READY_TEXT : WELCOME_TEXT);
    try {
      await this.runPromptLoop();
    } finally {
      await this.workflowController?.shutdown();
      this.scheduleController?.close();
      this.manager?.close();
      if (process.stdin.isTTY) process.stdin.setRawMode(false);
    }
  }

  private async initialize(): Promise<boolean> {
    try {
      const config = await this.configService.load();
      this.inputHistory = await PersistentInputHistory.create(config.globalDir);
      if (this.configService.hasConfiguredDefaultModel(config.models)) {
        await this.initializeRuntime();
      }
      return true;
    } catch (error) {
      console.error(error instanceof Error ? error.message : String(error));
      process.exitCode = 1;
      return false;
    }
  }

  private async initializeRuntime(): Promise<void> {
    this.manager = await AgentManager.create(this.configService, {
      requestPermission: (toolName, args, effect) =>
        this.permissionController.request(toolName, args, effect),
    });
    this.agent = this.manager.getActiveAgent();
    this.workflowRegistry = this.manager.getActiveWorkflowRegistry();
    this.scheduleController = ScheduleCliController.create(this.manager, {
      confirm: (prompt, details) =>
        this.permissionController.confirm(prompt, details),
    });
    await this.createActiveWorkflowController();
    this.scheduleController.showUnreadEvents();
  }

  private async createActiveWorkflowController(): Promise<void> {
    await this.workflowController?.shutdown();
    if (!this.manager || !this.agent || !this.workflowRegistry) return;
    this.workflowController = WorkflowCliController.create(
      this.agent,
      this.workflowRegistry,
      this.manager.globalDir,
      this.workflowUi(),
      this.manager.projectDir,
      this.manager.getActiveName(),
    );
  }

  private async runPromptLoop(): Promise<void> {
    for (;;) {
      const identity = this.manager?.getActiveName() ?? "main";
      const answer = await ghostPrompt({
        prompt: `[${identity}] > `,
        getCommands: () => this.getCommands(),
        history: this.inputHistory!.history,
      });
      if (answer === EOF) return;
      const line = answer.trim();
      if (line.length === 0) continue;
      await this.inputHistory?.record(line);
      if (await this.handleLine(line)) return;
    }
  }

  private getCommands(): string[] {
    return [
      ...BUILTIN_COMMANDS,
      ...(this.workflowRegistry?.list().map(
        (record) => record.definition.name,
      ) ?? []),
      ...(this.agent?.listSkillNames() ?? []),
    ];
  }

  private async handleLine(line: string): Promise<boolean> {
    if (!line.startsWith("/")) {
      await this.respondToUser(line);
      return false;
    }
    return this.handleCommand(line.slice(1).trim());
  }

  private async handleCommand(command: string): Promise<boolean> {
    if (command === "exit" || command === "quit") return true;
    if (command === "help") {
      this.showHelp();
      return false;
    }
    if (command === "agent" || command.startsWith("agent ")) {
      await this.handleAgentCommand(command.slice("agent".length).trim());
      return false;
    }
    if (command === "clear") {
      this.clearHistory();
      return false;
    }
    if (command === "model" || command.startsWith("model ")) {
      await this.handleModelCommand(command.slice("model".length).trim());
      return false;
    }
    if (command === "workflows") {
      this.showWorkflows();
      return false;
    }
    if (command === "workflow" || command.startsWith("workflow ")) {
      await this.getWorkflowControllerOrShowWelcome()?.handleWorkflowCommand(
        command.slice("workflow".length).trim(),
      );
      return false;
    }
    if (command === "runs") {
      this.getWorkflowControllerOrShowWelcome()?.showRuns();
      return false;
    }
    if (
      command === "workflow-debug" ||
      command.startsWith("workflow-debug ")
    ) {
      this.getWorkflowControllerOrShowWelcome()?.setDebugLogging(
        command.slice("workflow-debug".length).trim(),
      );
      return false;
    }
    if (command === "run" || command.startsWith("run ")) {
      await this.getWorkflowControllerOrShowWelcome()?.inspectRun(
        command.slice("run".length).trim(),
      );
      return false;
    }
    if (command === "resume" || command.startsWith("resume ")) {
      await this.getWorkflowControllerOrShowWelcome()?.resumeRun(
        command.slice("resume".length).trim(),
      );
      return false;
    }
    if (command === "cancel" || command.startsWith("cancel ")) {
      this.getWorkflowControllerOrShowWelcome()?.cancelRun(
        command.slice("cancel".length).trim(),
      );
      return false;
    }
    if (command === "schedules") {
      if (this.scheduleController) this.scheduleController.showSchedules();
      else console.log(WELCOME_TEXT);
      return false;
    }
    if (command === "schedule" || command.startsWith("schedule ")) {
      if (this.scheduleController) {
        await this.scheduleController.handleCommand(
          command.slice("schedule".length).trim(),
        );
      } else {
        console.log(WELCOME_TEXT);
      }
      return false;
    }

    const commandName = command.split(/\s/, 1)[0] ?? "";
    if (this.agent?.listSkillNames().includes(commandName)) {
      await this.handleSkillCommand(command);
      return false;
    }
    const workflow = this.workflowRegistry?.get(commandName);
    if (workflow && this.workflowController) {
      await this.workflowController.runWorkflow(
        workflow,
        command.slice(commandName.length).trim(),
      );
      return false;
    }
    await this.handleSkillCommand(command);
    return false;
  }

  private async handleAgentCommand(requested: string): Promise<void> {
    if (!this.manager) {
      console.log(WELCOME_TEXT);
      return;
    }
    if (requested.length === 0) {
      for (const agent of this.manager.listAgents()) {
        console.log(
          `  ${agent.name}${agent.active ? " (active)" : ""} — ${agent.description} [${agent.source}]`,
        );
      }
      return;
    }
    try {
      this.agent = await this.manager.switchAgent(requested);
      this.workflowRegistry = this.manager.getActiveWorkflowRegistry();
      await this.createActiveWorkflowController();
      console.log(`Switched to agent "${requested}".`);
    } catch (error) {
      console.log(error instanceof Error ? error.message : String(error));
    }
  }

  private clearHistory(): void {
    if (!this.manager) {
      console.log(WELCOME_TEXT);
      return;
    }
    this.manager.clearActiveHistory();
    console.log(`Cleared the ${this.manager.getActiveName()} conversation.`);
  }

  private showHelp(): void {
    console.log(HELP_TEXT);
    const skills = this.agent?.listSkillNames() ?? [];
    console.log(
      skills.length > 0 ? `Skills: ${skills.join(", ")}` : "No skills loaded.",
    );
  }

  private showWorkflows(): void {
    if (this.workflowController) {
      this.workflowController.showWorkflows();
      return;
    }
    console.log("No workflows discovered.");
  }

  private async handleModelCommand(requested: string): Promise<void> {
    if (!this.agent) {
      await this.setupFirstModel();
      return;
    }
    const current = this.agent.getCurrentModel();
    if (requested.length === 0) {
      console.log(`Current model: ${current.provider}/${current.model}`);
      console.log("Available:");
      for (const model of this.agent.listModels()) {
        console.log(
          `  ${model.provider}/${model.model}${model.active ? "  (active)" : ""}`,
        );
      }
      return;
    }
    const result = this.agent.setModel(requested);
    if (!result.ok) {
      console.log(result.error);
      return;
    }
    this.manager?.persistActive();
    const active = this.agent.getCurrentModel();
    console.log(
      result.changed
        ? `Switched model to "${active.provider}/${active.model}".`
        : `Already using "${active.provider}/${active.model}".`,
    );
  }

  private async setupFirstModel(): Promise<void> {
    const service = new ModelSetupService(
      this.configService,
      (prompt) => ghostPrompt({ prompt, getCommands: () => [] }),
      (message) => console.log(message),
    );
    const result = await service.run();
    if (result.status === "cancelled") {
      console.log("Setup cancelled. Use /model when you're ready.");
      return;
    }
    console.log(
      `Configured "${result.provider}/${result.model}" in ${result.configPath}.`,
    );
    try {
      await this.initializeRuntime();
      console.log(READY_TEXT);
    } catch (error) {
      console.error(
        `The model was saved, but the agent could not start: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
  }

  private async handleSkillCommand(command: string): Promise<void> {
    const agent = this.getAgentOrShowWelcome();
    if (!agent) return;
    const firstSpace = command.search(/\s/);
    const skillName = firstSpace === -1 ? command : command.slice(0, firstSpace);
    const promptText =
      firstSpace === -1 ? "" : command.slice(firstSpace + 1).trim();
    if (!agent.loadSkillByName(skillName)) {
      console.log(`Unknown command or skill: /${skillName}`);
      return;
    }
    console.log(`Loaded skill: ${skillName}`);
    if (promptText.length > 0) await this.respondToUser(promptText);
  }

  private async respondToUser(text: string): Promise<void> {
    const agent = this.getAgentOrShowWelcome();
    if (!agent) return;
    this.startActiveSpinner();
    try {
      console.log(await agent.handleUserMessage(text));
    } finally {
      this.stopSpinner();
      this.manager?.persistActive();
    }
  }

  private workflowUi(): WorkflowCliUi {
    return {
      pauseSpinner: () => this.pauseSpinner(),
      resumeSpinner: () => this.startActiveSpinner(),
      startSpinner: () => this.startActiveSpinner(),
      stopSpinner: () => this.stopSpinner(),
    };
  }

  private pauseSpinner(): boolean {
    const hadSpinner = this.stopActiveSpinner !== undefined;
    this.stopSpinner();
    return hadSpinner;
  }

  private startActiveSpinner(): void {
    this.stopActiveSpinner ??= startSpinner();
  }

  private stopSpinner(): void {
    this.stopActiveSpinner?.();
    this.stopActiveSpinner = undefined;
  }

  private getAgentOrShowWelcome(): Agent | undefined {
    if (!this.agent) console.log(WELCOME_TEXT);
    return this.agent;
  }

  private getWorkflowControllerOrShowWelcome():
    | WorkflowCliController
    | undefined {
    if (!this.workflowController) console.log(WELCOME_TEXT);
    return this.workflowController;
  }
}
