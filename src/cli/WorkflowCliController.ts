import type { Agent } from "#src/classes/Agent.js";
import { EOF, ghostPrompt } from "#src/ui/lineEditor.js";
import { WorkflowEngine } from "#src/workflows/WorkflowEngine.js";
import type { WorkflowRegistry } from "#src/workflows/WorkflowRegistry.js";
import { WorkflowRunStore } from "#src/workflows/WorkflowRunStore.js";
import { SerializedWorkflowHumanAdapter } from "#src/workflows/SerializedWorkflowHumanAdapter.js";
import type {
  JsonValue,
  WorkflowHumanAdapter,
  WorkflowHumanPrompt,
  WorkflowInvocationResult,
  WorkflowRecord,
} from "#src/workflows/types.js";

export interface WorkflowCliUi {
  pauseSpinner(): boolean;
  resumeSpinner(): void;
  startSpinner(): void;
  stopSpinner(): void;
}

export function buildWorkflowConfirmationDetails(
  record: WorkflowRecord,
  input: JsonValue,
): string {
  return (
    `${record.definition.description}\n\nInput:\n` +
    JSON.stringify(input, null, 2)
  );
}

export class WorkflowCliController {
  private debugLogging = false;

  private constructor(
    private readonly agent: Agent,
    private readonly registry: WorkflowRegistry,
    private readonly store: WorkflowRunStore,
    private readonly engine: WorkflowEngine,
    private readonly ui: WorkflowCliUi,
  ) {
    this.configureAgent();
  }

  static create(
    agent: Agent,
    registry: WorkflowRegistry,
    globalDir: string,
    ui: WorkflowCliUi,
    projectDir: string = process.cwd(),
    agentName = "main",
  ): WorkflowCliController {
    const store = new WorkflowRunStore(globalDir);
    try {
      const engine = new WorkflowEngine(
        agent,
        registry,
        store,
        projectDir,
        agentName,
      );
      return new WorkflowCliController(agent, registry, store, engine, ui);
    } catch (error) {
      store.close();
      throw error;
    }
  }

  async shutdown(): Promise<void> {
    await this.engine.shutdown();
    this.store.close();
  }

  showWorkflows(): void {
    const records = this.registry.list();
    if (records.length === 0) {
      console.log("No workflows discovered.");
      return;
    }
    console.log("Workflows:");
    for (const record of records) {
      const policy = record.definition.agentInvocation ?? "disabled";
      console.log(
        `  ${record.definition.name} (${record.source}, agent: ${policy}) — ` +
          record.definition.description,
      );
    }
  }

  async handleWorkflowCommand(command: string): Promise<void> {
    const firstSpace = command.search(/\s/);
    const name = firstSpace === -1 ? command : command.slice(0, firstSpace);
    const rawInput =
      firstSpace === -1 ? "" : command.slice(firstSpace + 1).trim();
    const record = this.registry.get(name);
    if (!record) {
      console.log(
        name.length === 0
          ? "Usage: /workflow <name> [input]"
          : `Unknown workflow: ${name}`,
      );
      return;
    }

    await this.runWorkflow(record, rawInput);
  }

  async runWorkflow(
    record: WorkflowRecord,
    rawInput: string,
  ): Promise<void> {
    try {
      const input = this.registry.parseInput(record, rawInput);
      const result = await this.engine.start(record.definition.name, input, {
        humanAdapter: this.createHumanAdapter(),
        onLog: (message, data) => this.printWorkflowLog(message, data),
      });
      await this.displayResult(result);
    } catch (error) {
      console.log(error instanceof Error ? error.message : String(error));
    }
  }

  showRuns(): void {
    const runs = this.engine.listRuns();
    if (runs.length === 0) {
      console.log("No workflow runs.");
      return;
    }
    for (const run of runs) {
      console.log(
        `${run.id}  ${run.status.padEnd(16)}  ` +
          `${run.agentName}/${run.workflowName}  ${run.updatedAt}`,
      );
    }
  }

  setDebugLogging(request: string): void {
    if (request === "on") this.debugLogging = true;
    else if (request === "off") this.debugLogging = false;
    else if (request !== "") {
      console.log("Usage: /workflow-debug [on|off]");
      return;
    }
    console.log(`Workflow debug logging is ${this.debugLogging ? "on" : "off"}.`);
  }

  async inspectRun(runId: string): Promise<void> {
    if (runId.length === 0) {
      console.log("Usage: /run <id>");
      return;
    }
    const run = this.engine.getRun(runId);
    if (!run) {
      console.log(`No workflow run found with id "${runId}".`);
      return;
    }
    await this.displayResult({
      run,
      presentation: run.presentation,
      ...(run.output === undefined ? {} : { value: run.output }),
    });
  }

  async resumeRun(runId: string): Promise<void> {
    if (runId.length === 0) {
      console.log("Usage: /resume <id>");
      return;
    }
    try {
      const result = await this.engine.resume(runId, {
        humanAdapter: this.createHumanAdapter(),
        onLog: (message, data) => this.printWorkflowLog(message, data),
      });
      await this.displayResult(result);
    } catch (error) {
      console.log(error instanceof Error ? error.message : String(error));
    }
  }

  cancelRun(runId: string): void {
    if (runId.length === 0) {
      console.log("Usage: /cancel <id>");
      return;
    }
    try {
      const run = this.engine.cancel(runId);
      console.log(`Workflow "${run.workflowName}" is ${run.status}.`);
    } catch (error) {
      console.log(error instanceof Error ? error.message : String(error));
    }
  }

  private configureAgent(): void {
    this.agent.configureWorkflows(this.registry.list(), {
      resolve: async (name) => {
        await this.registry.load();
        return this.registry.get(name);
      },
      invoke: async (record, input, signal) => {
        const result = await this.engine.start(record.definition.name, input, {
          expectedSourceFingerprint: record.fingerprint,
          humanAdapter: this.createHumanAdapter(),
          onLog: (message) => this.printWorkflowLog(message),
          ...(signal === undefined ? {} : { signal }),
        });
        if (result.run.status !== "completed" || result.value === undefined) {
          throw new Error(
            result.run.error ??
              `Workflow entered status "${result.run.status}" (${result.run.id}).`,
          );
        }
        return JSON.stringify({
          runId: result.run.id,
          workflow: result.run.workflowName,
          result: result.value,
        });
      },
      confirm: async (record, input) => {
        const response = await this.createHumanAdapter().request({
          kind: "approval",
          prompt: `Allow the agent to run workflow "${record.definition.name}"?`,
          details: buildWorkflowConfirmationDetails(record, input),
        });
        return response === true;
      },
    });
  }

  private async displayResult(
    result: WorkflowInvocationResult,
  ): Promise<void> {
    if (result.run.status !== "completed" || result.value === undefined) {
      const detail = result.run.error ? `: ${result.run.error}` : "";
      const statusText =
        result.run.status === "failed"
          ? "failed"
          : `is ${result.run.status}`;
      console.log(
        `Workflow "${result.run.workflowName}" ${statusText} ` +
          `(${result.run.id})${detail}`,
      );
      return;
    }

    if (result.presentation === "agent") {
      this.ui.startSpinner();
      try {
        console.log(
          await this.agent.presentWorkflowResult(
            result.run.workflowName,
            result.value,
          ),
        );
      } finally {
        this.ui.stopSpinner();
      }
      return;
    }

    console.log(
      typeof result.value === "string"
        ? result.value
        : JSON.stringify(result.value, null, 2),
    );
  }

  private createHumanAdapter(): WorkflowHumanAdapter {
    return new SerializedWorkflowHumanAdapter({
      request: (prompt) => this.promptHuman(prompt),
    });
  }

  private async promptHuman(prompt: WorkflowHumanPrompt): Promise<JsonValue> {
    const hadSpinner = this.ui.pauseSpinner();
    try {
      if (prompt.details) console.log(prompt.details);
      if (prompt.kind === "approval") {
        const answer = await ghostPrompt({
          prompt: `${prompt.prompt} [y/N] `,
          getCommands: () => [],
        });
        if (answer === EOF) return false;
        return ["y", "yes"].includes(answer.trim().toLowerCase());
      }

      if (prompt.kind === "choice") {
        return this.promptForChoice(prompt);
      }

      const answer = await ghostPrompt({
        prompt: `${prompt.prompt} `,
        getCommands: () => [],
      });
      if (answer === EOF) throw new Error("Human input was cancelled.");
      return answer;
    } finally {
      if (hadSpinner) this.ui.resumeSpinner();
    }
  }

  private async promptForChoice(
    prompt: WorkflowHumanPrompt,
  ): Promise<string> {
    const choices = prompt.choices ?? [];
    choices.forEach((choice, index) => {
      console.log(`  ${index + 1}. ${choice.label} (${choice.value})`);
      if (choice.description) console.log(`     ${choice.description}`);
    });

    for (;;) {
      const answer = await ghostPrompt({
        prompt: `${prompt.prompt} `,
        getCommands: () => [],
      });
      if (answer === EOF) throw new Error("Human input was cancelled.");
      const trimmed = answer.trim();
      const numeric = Number.parseInt(trimmed, 10);
      const byNumber =
        Number.isInteger(numeric) && numeric > 0
          ? choices[numeric - 1]
          : undefined;
      const byValue = choices.find((choice) => choice.value === trimmed);
      if (byNumber || byValue) return (byNumber ?? byValue)!.value;
      console.log("Choose one of the listed numbers or values.");
    }
  }

  private printWorkflowLog(
    message: string,
    data?: import("#src/workflows/types.js").JsonValue,
  ): void {
    if (!this.debugLogging) return;
    const hadSpinner = this.ui.pauseSpinner();
    const detail = data === undefined ? "" : ` ${JSON.stringify(data)}`;
    console.log(`[workflow] ${message}${detail}`);
    if (hadSpinner) this.ui.resumeSpinner();
  }
}
