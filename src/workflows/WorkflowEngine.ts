import { randomUUID } from "node:crypto";
import path from "node:path";
import { assertJsonValue } from "#src/workflows/schema.js";
import { isWorkflowOutput } from "#src/workflows/sdk.js";
import type {
  JsonValue,
  WorkflowHumanAdapter,
  WorkflowInvocationResult,
  WorkflowPresentation,
  WorkflowRunDetails,
  WorkflowRunSummary,
  WorkflowTrigger,
} from "#src/workflows/types.js";
import type { WorkflowAgentRuntime } from "#src/workflows/WorkflowAgentCoordinator.js";
import {
  WorkflowExecutionContext,
  WorkflowSuspendedError,
} from "#src/workflows/WorkflowExecutionContext.js";
import type { WorkflowRegistry } from "#src/workflows/WorkflowRegistry.js";
import type { WorkflowRunStore } from "#src/workflows/WorkflowRunStore.js";

export interface StartWorkflowOptions {
  humanAdapter?: WorkflowHumanAdapter;
  expectedSourceFingerprint?: string;
  signal?: AbortSignal;
  onLog?(message: string, data?: JsonValue): void;
  agentName?: string;
  trigger?: WorkflowTrigger;
  runId?: string;
}

export type ResumeWorkflowOptions = Omit<
  StartWorkflowOptions,
  "expectedSourceFingerprint"
>;

export class WorkflowEngine {
  private readonly controllers = new Map<string, AbortController>();
  private readonly activeExecutions = new Map<
    string,
    Promise<WorkflowInvocationResult>
  >();
  private shuttingDown = false;

  constructor(
    private readonly agent: WorkflowAgentRuntime,
    private readonly registry: WorkflowRegistry,
    private readonly store: WorkflowRunStore,
    private readonly projectDir = path.resolve(process.cwd()),
    private readonly agentName = "main",
  ) {}

  async start(
    name: string,
    input: JsonValue,
    options: StartWorkflowOptions = {},
  ): Promise<WorkflowInvocationResult> {
    this.assertAcceptingWork();
    await this.registry.load();
    this.assertAcceptingWork();
    const run = this.createRun(
      name,
      input,
      options.expectedSourceFingerprint,
      options.agentName ?? this.agentName,
      options.trigger ?? { type: "manual" },
      options.runId,
    );
    return this.execute(run.id, options);
  }

  async resume(
    runId: string,
    options: ResumeWorkflowOptions = {},
  ): Promise<WorkflowInvocationResult> {
    this.assertAcceptingWork();
    const run = this.requireProjectRun(runId);
    if (run.status === "completed") return this.toInvocationResult(run);
    if (
      run.status !== "waiting" &&
      run.status !== "interrupted" &&
      run.status !== "running" &&
      run.status !== "queued"
    ) {
      throw new Error(
        `Workflow run "${runId}" cannot be resumed from status "${run.status}".`,
      );
    }
    if (this.activeExecutions.has(runId)) {
      throw new Error(`Workflow run "${runId}" is already running.`);
    }
    if (run.status === "running") {
      this.store.transitionRunningStatus(runId, "interrupted");
    }
    return this.execute(runId, options);
  }

  getRun(runId: string): WorkflowRunDetails | undefined {
    const run = this.store.getRun(runId);
    return run?.projectDir === this.projectDir &&
      run.agentName === this.agentName
      ? run
      : undefined;
  }

  listRuns(): WorkflowRunSummary[] {
    return this.store.listRuns(this.projectDir, 50, this.agentName);
  }

  cancel(runId: string): WorkflowRunDetails {
    const run = this.requireProjectRun(runId);
    if (this.isTerminal(run.status)) return run;
    this.controllers
      .get(runId)
      ?.abort(new Error(`Workflow run "${runId}" was cancelled.`));
    this.store.updateStatus(runId, "cancelled");
    return this.store.getRun(runId)!;
  }

  async shutdown(): Promise<void> {
    this.shuttingDown = true;
    for (const runId of this.activeExecutions.keys()) {
      this.controllers.get(runId)?.abort();
      const run = this.store.getRun(runId);
      if (run?.status === "queued") {
        this.store.updateStatus(runId, "interrupted");
      } else {
        this.store.transitionRunningStatus(runId, "interrupted");
      }
    }
    const executions = Array.from(this.activeExecutions.values());
    if (executions.length === 0) return;
    let timeout: NodeJS.Timeout | undefined;
    await Promise.race([
      Promise.allSettled(executions),
      new Promise<void>((resolve) => {
        timeout = setTimeout(resolve, 2_000);
      }),
    ]);
    if (timeout) clearTimeout(timeout);
  }

  private createRun(
    name: string,
    input: JsonValue,
    expectedSourceFingerprint?: string,
    agentName = this.agentName,
    trigger: WorkflowTrigger = { type: "manual" },
    runId: string = randomUUID(),
  ): WorkflowRunDetails {
    const record = this.registry.get(name);
    if (!record) throw new Error(`Unknown workflow "${name}".`);
    if (
      expectedSourceFingerprint !== undefined &&
      record.fingerprint !== expectedSourceFingerprint
    ) {
      throw new Error(
        `Workflow "${name}" changed after its invocation was authorized. Try again.`,
      );
    }
    this.registry.validateInput(record, input);
    assertJsonValue(input, "Workflow input");

    return this.store.createRun({
      id: runId,
      workflowName: name,
      projectDir: this.projectDir,
      agentName,
      trigger,
      sourceEntryPath: record.entryPath,
      sourceFingerprint: record.fingerprint,
      presentation: record.definition.presentation ?? "direct",
      input,
    });
  }

  private async execute(
    runId: string,
    options: ResumeWorkflowOptions = {},
  ): Promise<WorkflowInvocationResult> {
    if (this.activeExecutions.has(runId)) {
      throw new Error(`Workflow run "${runId}" is already running.`);
    }

    const execution = this.executeInternal(runId, options);
    this.activeExecutions.set(runId, execution);
    try {
      return await execution;
    } catch (error) {
      const failure = error instanceof Error ? error : new Error(String(error));
      this.recordUnexpectedFailure(runId, failure, options.signal);
      throw error;
    } finally {
      this.activeExecutions.delete(runId);
    }
  }

  private async executeInternal(
    runId: string,
    options: ResumeWorkflowOptions,
  ): Promise<WorkflowInvocationResult> {
    await this.registry.load();
    this.assertAcceptingWork();
    const run = this.requireProjectRun(runId);
    if (this.isTerminal(run.status)) return this.toInvocationResult(run);
    const record = this.registry.get(run.workflowName);
    if (record?.fingerprint !== run.sourceFingerprint) {
      this.store.updateStatus(
        runId,
        "version-mismatch",
        "The workflow source changed after this run started.",
      );
      return this.toInvocationResult(this.store.getRun(runId)!);
    }

    const controller = new AbortController();
    const abortFromCaller = () => controller.abort();
    if (options.signal?.aborted) {
      abortFromCaller();
    } else {
      options.signal?.addEventListener("abort", abortFromCaller, {
        once: true,
      });
    }
    this.controllers.set(runId, controller);
    if (!this.store.transitionToRunning(runId)) {
      options.signal?.removeEventListener("abort", abortFromCaller);
      this.controllers.delete(runId);
      return this.toInvocationResult(this.store.getRun(runId)!);
    }

    try {
      controller.signal.throwIfAborted();
      const context = new WorkflowExecutionContext(this.agent, this.store, {
        runId,
        projectDir: this.projectDir,
        signal: controller.signal,
        ...(options.humanAdapter === undefined
          ? {}
          : { humanAdapter: options.humanAdapter }),
        ...(options.onLog === undefined ? {} : { onLog: options.onLog }),
      });
      const result = await context.execute(() =>
        record.definition.run(context, run.input),
      );
      controller.signal.throwIfAborted();
      const presentation: WorkflowPresentation = isWorkflowOutput(result)
        ? result.presentation
        : run.presentation;
      const value = isWorkflowOutput(result) ? result.value : result;
      assertJsonValue(value, "Workflow result");
      this.store.complete(runId, value, presentation);
    } catch (error) {
      if (controller.signal.aborted) {
        this.store.transitionRunningStatus(
          runId,
          this.shuttingDown ? "interrupted" : "cancelled",
        );
      } else if (error instanceof WorkflowSuspendedError) {
        this.store.transitionRunningStatus(runId, "waiting");
      } else {
        this.store.transitionRunningStatus(
          runId,
          "failed",
          error instanceof Error ? error.message : String(error),
        );
      }
    } finally {
      options.signal?.removeEventListener("abort", abortFromCaller);
      this.controllers.delete(runId);
    }

    return this.toInvocationResult(this.store.getRun(runId)!);
  }

  private toInvocationResult(
    run: WorkflowRunDetails,
  ): WorkflowInvocationResult {
    const result: WorkflowInvocationResult = {
      run,
      presentation: run.presentation,
    };
    if (run.output !== undefined) result.value = run.output;
    return result;
  }

  private requireProjectRun(runId: string): WorkflowRunDetails {
    const run = this.store.getRun(runId);
    if (
      run?.projectDir !== this.projectDir ||
      run.agentName !== this.agentName
    ) {
      const owner =
        run?.projectDir === this.projectDir
          ? ` It belongs to agent "${run.agentName}"; switch with "/agent ${run.agentName}".`
          : "";
      throw new Error(
        `No workflow run found with id "${runId}" for agent "${this.agentName}".${owner}`,
      );
    }
    return run;
  }

  private isTerminal(status: WorkflowRunDetails["status"]): boolean {
    return (
      status === "completed" ||
      status === "failed" ||
      status === "cancelled" ||
      status === "version-mismatch"
    );
  }

  private recordUnexpectedFailure(
    runId: string,
    error: Error,
    signal?: AbortSignal,
  ): void {
    const run = this.store.getRun(runId);
    if (!run || this.isTerminal(run.status)) return;
    let status: "interrupted" | "cancelled" | "failed";
    if (this.shuttingDown) {
      status = "interrupted";
    } else if (signal?.aborted) {
      status = "cancelled";
    } else {
      status = "failed";
    }
    this.store.updateStatus(
      runId,
      status,
      status === "failed" ? error.message : undefined,
    );
  }

  private assertAcceptingWork(): void {
    if (this.shuttingDown) {
      throw new Error("The workflow engine is shutting down.");
    }
  }
}
