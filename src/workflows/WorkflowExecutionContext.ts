import { createHash } from "node:crypto";
import { assertJsonValue } from "./schema.js";
import { workflowOutputApi } from "./sdk.js";
import type {
  ElevationFailure,
  ElevationOptions,
  HumanChoiceRequest,
  JsonValue,
  WorkflowAgentsApi,
  WorkflowCheckDetails,
  WorkflowCheckResult,
  WorkflowContext,
  WorkflowEffectOptions,
  WorkflowExecOptions,
  WorkflowExecResult,
  WorkflowHumanAdapter,
  WorkflowHumanApi,
  WorkflowHumanPrompt,
  WorkflowMapOptions,
} from "./types.js";
import {
  WorkflowAgentCoordinator,
  type WorkflowAgentRuntime,
} from "./WorkflowAgentCoordinator.js";
import { runWorkflowCommand } from "./WorkflowProcess.js";
import type {
  WorkflowRunStore,
  WorkflowStepKind,
} from "./WorkflowRunStore.js";

const STEP_KEY_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9._-]*$/;

function normalizeCheck(result: WorkflowCheckResult): WorkflowCheckDetails {
  return typeof result === "boolean" ? { passed: result } : result;
}

function toJsonValue(value: object): JsonValue {
  return structuredClone(value) as JsonValue;
}

export class WorkflowSuspendedError extends Error {}

export class WorkflowCheckFailedError extends Error {
  constructor(readonly failure: ElevationFailure<JsonValue>) {
    super("Workflow elevation attempts were exhausted.");
  }
}

export interface WorkflowExecutionContextOptions {
  runId: string;
  projectDir: string;
  signal: AbortSignal;
  humanAdapter?: WorkflowHumanAdapter;
  onLog?(message: string, data?: JsonValue): void;
}

export class WorkflowExecutionContext implements WorkflowContext {
  readonly output = workflowOutputApi;
  readonly agents: WorkflowAgentsApi;
  readonly human: WorkflowHumanApi;

  private readonly activeSteps = new Set<string>();
  private readonly humanOccurrences = new Map<string, number>();
  private readonly agentCoordinator: WorkflowAgentCoordinator;

  constructor(
    agent: WorkflowAgentRuntime,
    private readonly store: WorkflowRunStore,
    private readonly options: WorkflowExecutionContextOptions,
  ) {
    this.agentCoordinator = new WorkflowAgentCoordinator(
      agent,
      options.signal,
      options.onLog,
    );
    this.agents = this.agentCoordinator.api;
    this.human = {
      approve: (request) =>
        this.requestHuman({
          kind: "approval",
          prompt: request.prompt,
          ...(request.details === undefined
            ? {}
            : { details: request.details }),
        }).then((value) => {
          if (typeof value !== "boolean") {
            throw new TypeError("Human approval response must be a boolean.");
          }
          return value;
        }),
      choose: (request) => this.requestChoice(request),
      ask: (request) =>
        this.requestHuman({
          kind: "text",
          prompt: request.prompt,
          ...(request.description === undefined
            ? {}
            : { details: request.description }),
        }).then((value) => {
          if (typeof value !== "string") {
            throw new TypeError("Human text response must be a string.");
          }
          return value;
        }),
    };
  }

  get runId(): string {
    return this.options.runId;
  }

  get projectDir(): string {
    return this.options.projectDir;
  }

  get signal(): AbortSignal {
    return this.options.signal;
  }

  async execute<TValue>(operation: () => Promise<TValue>): Promise<TValue> {
    return operation();
  }

  async checkpoint<TValue extends JsonValue>(
    key: string,
    operation: () => Promise<TValue>,
  ): Promise<TValue> {
    this.assertStepKey(key);
    return this.runStep(key, "checkpoint", undefined, operation);
  }

  async effect<TValue extends JsonValue>(
    key: string,
    options: WorkflowEffectOptions<TValue>,
  ): Promise<TValue> {
    this.assertStepKey(key);
    if (options.idempotencyKey.trim().length === 0) {
      throw new Error("Effect idempotencyKey cannot be empty.");
    }
    const input = { idempotencyKey: options.idempotencyKey };
    return this.runStep(
      key,
      "effect",
      input,
      () =>
        options.run({
          idempotencyKey: options.idempotencyKey,
          signal: this.signal,
        }),
    );
  }

  exec(
    command: string,
    args: string[] = [],
    options: WorkflowExecOptions = {},
  ): Promise<WorkflowExecResult> {
    if (command.trim().length === 0) {
      throw new Error("Command cannot be empty.");
    }
    return runWorkflowCommand(
      this.projectDir,
      this.signal,
      command,
      args,
      options,
    );
  }

  async map<TItem, TResult>(
    items: readonly TItem[],
    options: WorkflowMapOptions<TItem, TResult>,
  ): Promise<TResult[]> {
    const concurrency = options.concurrency ?? 4;
    if (!Number.isInteger(concurrency) || concurrency < 1) {
      throw new Error("Map concurrency must be a positive integer.");
    }
    const results = new Array<TResult>(items.length);
    let nextIndex = 0;
    let failure: Error | undefined;

    const worker = async (): Promise<void> => {
      while (!failure) {
        const index = nextIndex;
        nextIndex += 1;
        const item = items[index];
        if (item === undefined && index >= items.length) return;
        try {
          this.signal.throwIfAborted();
          results[index] = await options.run(item!, index);
        } catch (error) {
          failure =
            error instanceof Error ? error : new Error(String(error));
        }
      }
    };

    const workerCount = Math.min(concurrency, items.length);
    await Promise.all(
      Array.from({ length: workerCount }, () => worker()),
    );
    if (failure) throw failure;
    return results;
  }

  async elevate<TValue extends JsonValue, TFallback extends JsonValue = TValue>(
    options: ElevationOptions<TValue, TFallback>,
  ): Promise<TValue | TFallback> {
    if (!Number.isInteger(options.attempts) || options.attempts < 1) {
      throw new Error("Elevation attempts must be a positive integer.");
    }

    const session = await this.agentCoordinator.resolveElevationSession(
      options.model,
      options.context,
    );
    const results: TValue[] = [];
    const checks: WorkflowCheckDetails[] = [];

    for (let attempt = 1; attempt <= options.attempts; attempt += 1) {
      this.signal.throwIfAborted();
      const value = await options.operation({
        attempt,
        previousResults: [...results],
        session,
      });
      const check = normalizeCheck(await options.check(value));
      results.push(value);
      checks.push(check);
      if (check.passed) return value;
    }

    const failure: ElevationFailure<TValue> = {
      results,
      checks,
      session,
    };
    if (options.fallback) return options.fallback(failure);
    throw new WorkflowCheckFailedError(
      failure as ElevationFailure<JsonValue>,
    );
  }

  async log(message: string, data?: JsonValue): Promise<void> {
    this.signal.throwIfAborted();
    this.options.onLog?.(message, data);
  }

  private async runStep<TValue extends JsonValue>(
    key: string,
    kind: WorkflowStepKind,
    input: JsonValue | undefined,
    operation: () => Promise<TValue>,
  ): Promise<TValue> {
    this.signal.throwIfAborted();
    if (this.activeSteps.has(key)) {
      throw new Error(`Workflow step "${key}" is already running.`);
    }

    const existing = this.store.getStep(this.runId, key);
    if (existing && existing.kind !== kind) {
      throw new Error(
        `Workflow step "${key}" was previously used as ${existing.kind}.`,
      );
    }
    if (
      existing &&
      JSON.stringify(existing.input ?? null) !== JSON.stringify(input ?? null)
    ) {
      throw new Error(`Workflow step "${key}" changed its input.`);
    }
    if (existing?.state === "completed") {
      if (existing.output === undefined) {
        throw new Error(`Workflow step "${key}" has no stored output.`);
      }
      return existing.output as TValue;
    }
    if (!existing) this.store.startStep(this.runId, key, kind, input);

    this.activeSteps.add(key);
    try {
      const value = await operation();
      assertJsonValue(value, `Workflow step "${key}" output`);
      this.store.completeStep(this.runId, key, value);
      return value;
    } finally {
      this.activeSteps.delete(key);
    }
  }

  private async requestChoice(request: HumanChoiceRequest): Promise<string> {
    if (request.choices.length === 0) {
      throw new Error("A human choice request needs at least one choice.");
    }
    const value = await this.requestHuman({
      kind: "choice",
      prompt: request.prompt,
      choices: request.choices,
    });
    if (
      typeof value !== "string" ||
      !request.choices.some((choice) => choice.value === value)
    ) {
      throw new Error("Human choice response is not one of the allowed values.");
    }
    return value;
  }

  private async requestHuman(prompt: WorkflowHumanPrompt): Promise<JsonValue> {
    this.signal.throwIfAborted();
    const serialized = JSON.stringify(prompt);
    const promptHash = createHash("sha256")
      .update(serialized)
      .digest("hex")
      .slice(0, 16);
    const occurrence = this.humanOccurrences.get(promptHash) ?? 0;
    this.humanOccurrences.set(promptHash, occurrence + 1);
    const key = `human.${promptHash}.${occurrence}`;
    const existing = this.store.getStep(this.runId, key);

    if (existing?.kind !== undefined && existing.kind !== "human") {
      throw new Error(`Workflow human step "${key}" is invalid.`);
    }
    if (existing?.state === "completed") {
      if (existing.output === undefined) {
        throw new Error("Stored human response is missing.");
      }
      return existing.output;
    }
    if (!existing) {
      this.store.startStep(
        this.runId,
        key,
        "human",
        toJsonValue(prompt),
      );
    }
    if (!this.options.humanAdapter) throw new WorkflowSuspendedError();

    const response = await this.options.humanAdapter.request(prompt);
    assertJsonValue(response, "Human response");
    this.store.completeStep(this.runId, key, response);
    return response;
  }

  private assertStepKey(key: string): void {
    if (!STEP_KEY_PATTERN.test(key)) {
      throw new Error(
        `Workflow step key "${key}" must contain only letters, numbers, ".", "_" or "-".`,
      );
    }
  }
}
