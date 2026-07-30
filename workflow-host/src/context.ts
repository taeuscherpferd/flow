import {
  assertPositiveInteger,
  choiceToJson,
  compactOptions,
  readCheckDetails,
  readExecResult,
  readModel,
  requireObject,
  requireString,
} from "./contextValues.js";
import { assertJsonValue } from "./json.js";
import { RpcFailure, type RpcConnection } from "./rpc.js";
import { workflowOutputApi } from "./sdk.js";
import type {
  ElevationOptions,
  JsonValue,
  ModelRef,
  WorkflowAgentResponse,
  WorkflowAgentRunOptions,
  WorkflowAgentSession,
  WorkflowContext,
  WorkflowEffectOptions,
  WorkflowExecOptions,
  WorkflowExecResult,
  WorkflowMapOptions,
} from "./types.js";

type RegisteredCallback = (
  argumentsValue: JsonValue[],
) => Promise<JsonValue>;

export class CallbackRegistry {
  private readonly callbacks = new Map<string, RegisteredCallback>();
  private nextId = 1;

  register(callback: RegisteredCallback): string {
    const id = `callback-${this.nextId}`;
    this.nextId += 1;
    this.callbacks.set(id, callback);
    return id;
  }

  remove(id: string): void {
    this.callbacks.delete(id);
  }

  async invoke(id: string, argumentsValue: JsonValue[]): Promise<JsonValue> {
    const callback = this.callbacks.get(id);
    if (!callback) {
      throw new RpcFailure(-32602, `Unknown or expired callback "${id}".`);
    }
    return callback(argumentsValue);
  }
}

class WorkflowAgentSessionProxy implements WorkflowAgentSession {
  constructor(
    private readonly runtime: WorkflowContextRuntime,
    readonly id: string,
    readonly model: ModelRef,
    private readonly runDefaults: WorkflowAgentRunOptions = {},
  ) {}

  async run(
    prompt: string,
    options: WorkflowAgentRunOptions = {},
  ): Promise<WorkflowAgentResponse> {
    const thinking = options.thinking ?? this.runDefaults.thinking;
    const value = await this.runtime.request("sdk.agent.run", {
      runId: this.runtime.runId,
      sessionId: this.id,
      prompt,
      options: compactOptions({
        tools: options.tools ?? this.runDefaults.tools ?? "default",
        ...(thinking === undefined ? {} : { thinking }),
      }),
    });
    const response = requireObject(value, "Agent response");
    return {
      content: requireString(response["content"], "Agent response content"),
      model: readModel(response["model"]),
    };
  }
}

interface WorkflowContextRuntimeOptions {
  runId: string;
  projectDir: string;
  signal: AbortSignal;
  connection: RpcConnection;
  callbacks: CallbackRegistry;
}

class WorkflowContextRuntime {
  readonly runId: string;
  readonly projectDir: string;
  readonly signal: AbortSignal;
  readonly connection: RpcConnection;
  readonly callbacks: CallbackRegistry;

  constructor(options: WorkflowContextRuntimeOptions) {
    this.runId = options.runId;
    this.projectDir = options.projectDir;
    this.signal = options.signal;
    this.connection = options.connection;
    this.callbacks = options.callbacks;
  }

  request(method: string, params: JsonValue): Promise<JsonValue> {
    this.signal.throwIfAborted();
    return new Promise<JsonValue>((resolve, reject) => {
      const abort = (): void => {
        reject(
          this.signal.reason instanceof Error
            ? this.signal.reason
            : new Error(`Workflow run "${this.runId}" was cancelled.`),
        );
      };
      this.signal.addEventListener("abort", abort, { once: true });
      void this.connection.request(method, params).then(
        (value) => {
          this.signal.removeEventListener("abort", abort);
          resolve(value);
        },
        (error) => {
          this.signal.removeEventListener("abort", abort);
          reject(error);
        },
      );
    });
  }

  session(
    value: JsonValue,
    runDefaults: WorkflowAgentRunOptions = {},
  ): WorkflowAgentSession {
    const descriptor = requireObject(value, "Agent session");
    return new WorkflowAgentSessionProxy(
      this,
      requireString(descriptor["id"], "Agent session id"),
      readModel(descriptor["model"]),
      runDefaults,
    );
  }
}

export function createWorkflowContext(
  options: WorkflowContextRuntimeOptions,
): WorkflowContext {
  const runtime = new WorkflowContextRuntime(options);

  return {
    runId: runtime.runId,
    projectDir: runtime.projectDir,
    signal: runtime.signal,
    output: workflowOutputApi,
    agents: {
      create: async (createOptions) => {
        const value = await runtime.request("sdk.agent.create", {
          runId: runtime.runId,
          ...(createOptions.model === undefined
            ? {}
            : { model: createOptions.model }),
        });
        return runtime.session(value);
      },
      fork: async (session, forkOptions = {}) => {
        const value = await runtime.request("sdk.agent.fork", {
          runId: runtime.runId,
          sessionId: session.id,
          ...(forkOptions.model === undefined
            ? {}
            : { model: forkOptions.model }),
        });
        return runtime.session(value);
      },
    },
    human: {
      approve: async (request) => {
        const value = await runtime.request("sdk.human", {
          runId: runtime.runId,
          kind: "approval",
          prompt: request.prompt,
          ...(request.details === undefined
            ? {}
            : { details: request.details }),
        });
        if (typeof value !== "boolean") {
          throw new TypeError("Human approval response must be a boolean.");
        }
        return value;
      },
      choose: async (request) => {
        const value = await runtime.request("sdk.human", {
          runId: runtime.runId,
          kind: "choice",
          prompt: request.prompt,
          choices: request.choices.map(choiceToJson),
        });
        if (
          typeof value !== "string" ||
          !request.choices.some((choice) => choice.value === value)
        ) {
          throw new TypeError(
            "Human choice response is not one of the allowed values.",
          );
        }
        return value;
      },
      ask: async (request) => {
        const value = await runtime.request("sdk.human", {
          runId: runtime.runId,
          kind: "text",
          prompt: request.prompt,
          ...(request.description === undefined
            ? {}
            : { details: request.description }),
        });
        return requireString(value, "Human text response");
      },
    },
    checkpoint: async <TValue extends JsonValue>(
      key: string,
      operation: () => Promise<TValue>,
    ) => {
      const callbackId = runtime.callbacks.register(async () => {
        const value = await operation();
        assertJsonValue(value, `Checkpoint "${key}" output`);
        return value;
      });
      try {
        return (await runtime.request("sdk.checkpoint", {
          runId: runtime.runId,
          key,
          callbackId,
        })) as TValue;
      } finally {
        runtime.callbacks.remove(callbackId);
      }
    },
    effect: async <TValue extends JsonValue>(
      key: string,
      effectOptions: WorkflowEffectOptions<TValue>,
    ) => {
      const callbackId = runtime.callbacks.register(async () => {
        const value = await effectOptions.run({
          idempotencyKey: effectOptions.idempotencyKey,
          signal: runtime.signal,
        });
        assertJsonValue(value, `Effect "${key}" output`);
        return value;
      });
      try {
        return (await runtime.request("sdk.effect", {
          runId: runtime.runId,
          key,
          idempotencyKey: effectOptions.idempotencyKey,
          callbackId,
        })) as TValue;
      } finally {
        runtime.callbacks.remove(callbackId);
      }
    },
    exec: async (
      command: string,
      args: string[] = [],
      execOptions: WorkflowExecOptions = {},
    ) => {
      const value = await runtime.request("sdk.exec", {
        runId: runtime.runId,
        command,
        args,
        options: compactOptions(execOptions),
      });
      return readExecResult(value);
    },
    map: async <TItem extends JsonValue, TResult extends JsonValue>(
      items: readonly TItem[],
      mapOptions: WorkflowMapOptions<TItem, TResult>,
    ) => {
      const concurrency = mapOptions.concurrency ?? 4;
      assertPositiveInteger(concurrency, "Map concurrency");
      items.forEach((item, index) =>
        assertJsonValue(item, `Map item ${index}`),
      );
      const callbackId = runtime.callbacks.register(async (args) => {
        const item = args[0] as TItem;
        const index = args[1];
        if (typeof index !== "number") {
          throw new TypeError("Map callback index must be a number.");
        }
        const value = await mapOptions.run(item, index);
        assertJsonValue(value, "Map callback output");
        return value;
      });
      try {
        const value = await runtime.request("sdk.map", {
          runId: runtime.runId,
          items: [...items],
          concurrency,
          callbackId,
        });
        if (!Array.isArray(value)) {
          throw new TypeError("Map response must be an array.");
        }
        return value as TResult[];
      } finally {
        runtime.callbacks.remove(callbackId);
      }
    },
    elevate: <TValue extends JsonValue, TFallback extends JsonValue = TValue>(
      elevationOptions: ElevationOptions<TValue, TFallback>,
    ) => elevate(runtime, elevationOptions),
    log: async (message: string, data?: JsonValue) => {
      await runtime.request("sdk.log", {
        runId: runtime.runId,
        message,
        ...(data === undefined ? {} : { data }),
      });
    },
  };
}

async function elevate<
  TValue extends JsonValue,
  TFallback extends JsonValue,
>(
  runtime: WorkflowContextRuntime,
  options: ElevationOptions<TValue, TFallback>,
): Promise<TValue | TFallback> {
  assertPositiveInteger(options.attempts, "Elevation attempts");
  const operationId = runtime.callbacks.register(async (args) => {
    const attempt = requireObject(args[0] ?? null, "Elevation attempt");
    const attemptNumber = attempt["attempt"];
    const previousResults = attempt["previousResults"];
    if (typeof attemptNumber !== "number" || !Array.isArray(previousResults)) {
      throw new TypeError("Rust sent an invalid elevation attempt.");
    }
    const value = await options.operation({
      attempt: attemptNumber,
      previousResults: previousResults as TValue[],
      session: runtime.session(
        attempt["session"] ?? null,
        options.thinking === undefined
          ? {}
          : { thinking: options.thinking },
      ),
    });
    assertJsonValue(value, "Elevation operation output");
    return value;
  });
  const checkId = runtime.callbacks.register(async (args) => {
    const result = await options.check(args[0] as TValue);
    assertJsonValue(result as JsonValue, "Elevation check output");
    return result as JsonValue;
  });
  const fallbackId =
    options.fallback === undefined
      ? undefined
      : runtime.callbacks.register(async (args) => {
          const failure = requireObject(
            args[0] ?? null,
            "Elevation failure",
          );
          const results = failure["results"];
          const checks = failure["checks"];
          if (!Array.isArray(results) || !Array.isArray(checks)) {
            throw new TypeError("Rust sent an invalid elevation failure.");
          }
          const value = await options.fallback!({
            results: results as TValue[],
            checks: readCheckDetails(checks),
            session: runtime.session(
              failure["session"] ?? null,
              options.thinking === undefined
                ? {}
                : { thinking: options.thinking },
            ),
          });
          assertJsonValue(value, "Elevation fallback output");
          return value;
        });
  try {
    return (await runtime.request("sdk.elevate", {
      runId: runtime.runId,
      model: options.model,
      ...(options.thinking === undefined
        ? {}
        : { thinking: options.thinking }),
      attempts: options.attempts,
      context:
        options.context.mode === "fresh"
          ? { mode: "fresh" }
          : {
              mode: options.context.mode,
              sessionId: options.context.session.id,
            },
      operationCallbackId: operationId,
      checkCallbackId: checkId,
      ...(fallbackId === undefined
        ? {}
        : { fallbackCallbackId: fallbackId }),
    })) as TValue | TFallback;
  } finally {
    runtime.callbacks.remove(operationId);
    runtime.callbacks.remove(checkId);
    if (fallbackId !== undefined) runtime.callbacks.remove(fallbackId);
  }
}
