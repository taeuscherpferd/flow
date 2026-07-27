import type { ModelRef } from "#src/classes/Agent.js";
import type { ThinkingMode } from "#src/providers/types.js";

export type JsonPrimitive = string | number | boolean | null;

export type JsonValue =
  | JsonPrimitive
  | JsonValue[]
  | { [key: string]: JsonValue };

export type JsonObject = { [key: string]: JsonValue };

export type WorkflowPresentation = "direct" | "agent";

export type AgentInvocationPolicy = "disabled" | "confirm" | "automatic";

export type WorkflowRunStatus =
  | "queued"
  | "running"
  | "waiting"
  | "interrupted"
  | "completed"
  | "failed"
  | "cancelled"
  | "version-mismatch";

export type WorkflowSchema =
  | {
      type: "string";
      description?: string;
      enum?: string[];
      minLength?: number;
    }
  | {
      type: "number";
      description?: string;
      minimum?: number;
      maximum?: number;
    }
  | {
      type: "boolean";
      description?: string;
    }
  | {
      type: "array";
      description?: string;
      items: WorkflowSchema;
    }
  | {
      type: "object";
      description?: string;
      properties: Record<string, WorkflowSchema>;
      required?: string[];
      additionalProperties?: boolean;
    };

export type WorkflowRootSchema = Extract<
  WorkflowSchema,
  { type: "string" | "object" }
>;

export interface WorkflowInputDefinition {
  schema: WorkflowRootSchema;
}

export interface WorkflowOutputValue<TValue = JsonValue> {
  readonly kind: "workflow-output";
  readonly presentation: WorkflowPresentation;
  readonly value: TValue;
}

export type WorkflowRunResult<TValue = JsonValue> =
  | TValue
  | WorkflowOutputValue<TValue>;

export interface WorkflowDefinition<
  TInput = string,
  TOutput = JsonValue,
> {
  name: string;
  description: string;
  input?: WorkflowInputDefinition;
  agentInvocation?: AgentInvocationPolicy;
  presentation?: WorkflowPresentation;
  run(
    context: WorkflowContext,
    input: TInput,
  ): Promise<WorkflowRunResult<TOutput>>;
}

export interface WorkflowRecord {
  definition: WorkflowDefinition<JsonValue, JsonValue>;
  directory: string;
  entryPath: string;
  fingerprint: string;
  source: "global" | "project";
  agentName?: string;
  resourceId?: string;
}

export interface WorkflowOutputApi {
  direct<TValue extends JsonValue>(
    value: TValue,
  ): WorkflowOutputValue<TValue>;
  agent<TValue extends JsonValue>(
    value: TValue,
  ): WorkflowOutputValue<TValue>;
}

export interface WorkflowAgentRunOptions {
  tools?: "default" | "none";
  thinking?: ThinkingMode;
}

export interface WorkflowAgentResponse {
  content: string;
  model: ModelRef;
}

export interface WorkflowAgentSession {
  readonly id: string;
  readonly model: ModelRef;
  run(
    prompt: string,
    options?: WorkflowAgentRunOptions,
  ): Promise<WorkflowAgentResponse>;
}

export interface WorkflowAgentCreateOptions {
  model?: string;
}

export interface WorkflowAgentForkOptions {
  model?: string;
}

export interface WorkflowAgentsApi {
  create(options: WorkflowAgentCreateOptions): Promise<WorkflowAgentSession>;
  fork(
    session: WorkflowAgentSession,
    options?: WorkflowAgentForkOptions,
  ): Promise<WorkflowAgentSession>;
}

export type ElevationContext =
  | { mode: "fresh" }
  | { mode: "reuse"; session: WorkflowAgentSession }
  | { mode: "fork"; session: WorkflowAgentSession };

export interface WorkflowCheckDetails {
  passed: boolean;
  message?: string;
  data?: JsonValue;
}

export type WorkflowCheckResult = boolean | WorkflowCheckDetails;

export interface ElevationAttempt<TValue extends JsonValue> {
  attempt: number;
  previousResults: TValue[];
  session: WorkflowAgentSession;
}

export interface ElevationFailure<TValue extends JsonValue> {
  results: TValue[];
  checks: WorkflowCheckDetails[];
  session: WorkflowAgentSession;
}

export interface ElevationOptions<
  TValue extends JsonValue,
  TFallback extends JsonValue = TValue,
> {
  model: string;
  thinking?: ThinkingMode;
  attempts: number;
  context: ElevationContext;
  operation(attempt: ElevationAttempt<TValue>): Promise<TValue>;
  check(value: TValue): WorkflowCheckResult | Promise<WorkflowCheckResult>;
  fallback?(
    failure: ElevationFailure<TValue>,
  ): Promise<TFallback>;
}

export interface HumanApprovalRequest {
  prompt: string;
  details?: string;
}

export interface HumanChoice {
  value: string;
  label: string;
  description?: string;
}

export interface HumanChoiceRequest {
  prompt: string;
  choices: HumanChoice[];
}

export interface HumanTextRequest {
  prompt: string;
  description?: string;
}

export interface WorkflowHumanApi {
  approve(request: HumanApprovalRequest): Promise<boolean>;
  choose(request: HumanChoiceRequest): Promise<string>;
  ask(request: HumanTextRequest): Promise<string>;
}

export interface WorkflowExecOptions {
  cwd?: string;
  env?: Record<string, string>;
  input?: string;
  timeoutMs?: number;
  maxOutputBytes?: number;
  allowFailure?: boolean;
}

export interface WorkflowExecResult {
  command: string;
  args: string[];
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface WorkflowMapOptions<TItem, TResult> {
  concurrency?: number;
  run(item: TItem, index: number): Promise<TResult>;
}

export interface WorkflowEffectContext {
  readonly idempotencyKey: string;
  readonly signal: AbortSignal;
}

export interface WorkflowEffectOptions<TValue extends JsonValue> {
  idempotencyKey: string;
  run(context: WorkflowEffectContext): Promise<TValue>;
}

export interface WorkflowContext {
  readonly runId: string;
  readonly projectDir: string;
  readonly signal: AbortSignal;
  readonly output: WorkflowOutputApi;
  readonly agents: WorkflowAgentsApi;
  readonly human: WorkflowHumanApi;
  checkpoint<TValue extends JsonValue>(
    key: string,
    operation: () => Promise<TValue>,
  ): Promise<TValue>;
  effect<TValue extends JsonValue>(
    key: string,
    options: WorkflowEffectOptions<TValue>,
  ): Promise<TValue>;
  exec(
    command: string,
    args?: string[],
    options?: WorkflowExecOptions,
  ): Promise<WorkflowExecResult>;
  map<TItem, TResult>(
    items: readonly TItem[],
    options: WorkflowMapOptions<TItem, TResult>,
  ): Promise<TResult[]>;
  elevate<TValue extends JsonValue, TFallback extends JsonValue = TValue>(
    options: ElevationOptions<TValue, TFallback>,
  ): Promise<TValue | TFallback>;
  log(message: string, data?: JsonValue): Promise<void>;
}

export interface WorkflowRunSummary {
  id: string;
  workflowName: string;
  projectDir: string;
  agentName: string;
  trigger: WorkflowTrigger;
  status: WorkflowRunStatus;
  presentation: WorkflowPresentation;
  createdAt: string;
  updatedAt: string;
  error?: string;
}

export type WorkflowTrigger =
  | { type: "manual" }
  | { type: "schedule"; scheduleId: string; scheduledFor: string };

export interface WorkflowRunDetails extends WorkflowRunSummary {
  input: JsonValue;
  output?: JsonValue;
  sourceFingerprint: string;
}

export interface WorkflowInvocationResult {
  run: WorkflowRunDetails;
  presentation: WorkflowPresentation;
  value?: JsonValue;
}

export interface WorkflowHumanPrompt {
  kind: "approval" | "choice" | "text";
  prompt: string;
  details?: string;
  choices?: HumanChoice[];
}

export interface WorkflowHumanAdapter {
  request(prompt: WorkflowHumanPrompt): Promise<JsonValue>;
}
