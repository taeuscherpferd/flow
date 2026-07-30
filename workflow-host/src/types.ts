export type JsonPrimitive = string | number | boolean | null;

export type JsonValue =
  | JsonPrimitive
  | JsonValue[]
  | { [key: string]: JsonValue };

export type JsonObject = { [key: string]: JsonValue };

export type WorkflowPresentation = "direct" | "agent";
export type AgentInvocationPolicy = "disabled" | "confirm" | "automatic";
export type WorkflowThinking =
  | "default"
  | "off"
  | "on"
  | "low"
  | "medium"
  | "high";

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

export interface WorkflowOutputValue<TValue = JsonValue> {
  readonly kind: "workflow-output";
  readonly presentation: WorkflowPresentation;
  readonly value: TValue;
}

export interface WorkflowDefinition<TInput = string, TOutput = JsonValue> {
  name: string;
  description: string;
  input?: { schema: WorkflowRootSchema };
  agentInvocation?: AgentInvocationPolicy;
  presentation?: WorkflowPresentation;
  run(
    context: WorkflowContext,
    input: TInput,
  ): Promise<TOutput | WorkflowOutputValue<TOutput>>;
}

export interface WorkflowOutputApi {
  direct<TValue extends JsonValue>(
    value: TValue,
  ): WorkflowOutputValue<TValue>;
  agent<TValue extends JsonValue>(
    value: TValue,
  ): WorkflowOutputValue<TValue>;
}

export interface ModelRef {
  provider: string;
  model: string;
  active: boolean;
}

export interface WorkflowAgentRunOptions {
  tools?: "default" | "none";
  thinking?: WorkflowThinking;
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

export interface WorkflowAgentsApi {
  create(options: { model?: string }): Promise<WorkflowAgentSession>;
  fork(
    session: WorkflowAgentSession,
    options?: { model?: string },
  ): Promise<WorkflowAgentSession>;
}

export interface HumanChoice {
  value: string;
  label: string;
  description?: string;
}

export interface WorkflowHumanApi {
  approve(request: {
    prompt: string;
    details?: string;
  }): Promise<boolean>;
  choose(request: {
    prompt: string;
    choices: HumanChoice[];
  }): Promise<string>;
  ask(request: {
    prompt: string;
    description?: string;
  }): Promise<string>;
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

export interface WorkflowEffectOptions<TValue extends JsonValue> {
  idempotencyKey: string;
  run(context: {
    readonly idempotencyKey: string;
    readonly signal: AbortSignal;
  }): Promise<TValue>;
}

export interface WorkflowCheckDetails {
  passed: boolean;
  message?: string;
  data?: JsonValue;
}

export type WorkflowCheckResult = boolean | WorkflowCheckDetails;

export type ElevationContext =
  | { mode: "fresh" }
  | { mode: "reuse"; session: WorkflowAgentSession }
  | { mode: "fork"; session: WorkflowAgentSession };

export interface ElevationOptions<
  TValue extends JsonValue,
  TFallback extends JsonValue = TValue,
> {
  model: string;
  thinking?: WorkflowThinking;
  attempts: number;
  context: ElevationContext;
  operation(attempt: {
    attempt: number;
    previousResults: TValue[];
    session: WorkflowAgentSession;
  }): Promise<TValue>;
  check(
    value: TValue,
  ): WorkflowCheckResult | Promise<WorkflowCheckResult>;
  fallback?(failure: {
    results: TValue[];
    checks: WorkflowCheckDetails[];
    session: WorkflowAgentSession;
  }): Promise<TFallback>;
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
  map<TItem extends JsonValue, TResult extends JsonValue>(
    items: readonly TItem[],
    options: WorkflowMapOptions<TItem, TResult>,
  ): Promise<TResult[]>;
  elevate<TValue extends JsonValue, TFallback extends JsonValue = TValue>(
    options: ElevationOptions<TValue, TFallback>,
  ): Promise<TValue | TFallback>;
  log(message: string, data?: JsonValue): Promise<void>;
}
