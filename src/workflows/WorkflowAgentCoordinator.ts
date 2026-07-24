import type { ChatMessage, ModelProvider } from "#src/providers/types.js";
import type {
  ElevationContext,
  JsonValue,
  WorkflowAgentCreateOptions,
  WorkflowAgentForkOptions,
  WorkflowAgentResponse,
  WorkflowAgentRunOptions,
  WorkflowAgentSession,
  WorkflowAgentsApi,
} from "#src/workflows/types.js";

interface RuntimeAgentSessionState {
  tail: Promise<void>;
}

function mergeRunOptions(
  defaults: WorkflowAgentRunOptions,
  options?: WorkflowAgentRunOptions,
): WorkflowAgentRunOptions {
  const thinking = options?.thinking ?? defaults.thinking;
  return {
    tools: options?.tools ?? defaults.tools ?? "default",
    ...(thinking === undefined ? {} : { thinking }),
  };
}

export interface WorkflowAgentSessionRuntime {
  readonly id: string;
  getModel(): { provider: string; model: string };
  run(
    prompt: string,
    options?: WorkflowAgentRunOptions,
    signal?: AbortSignal,
  ): Promise<string>;
  snapshotHistory(): ChatMessage[];
  restoreHistory(history: ChatMessage[]): void;
  retarget(
    providerName: string,
    provider: ModelProvider,
    model: string,
    contextWindow: number,
  ): void;
}

export interface WorkflowAgentRuntime {
  createSession(
    modelSpec: string,
    history?: ChatMessage[],
    sessionId?: string,
  ): WorkflowAgentSessionRuntime;
  forkSession(
    session: WorkflowAgentSessionRuntime,
    modelSpec?: string,
  ): WorkflowAgentSessionRuntime;
  retargetSession(
    session: WorkflowAgentSessionRuntime,
    modelSpec: string,
  ): void;
}

class RuntimeAgentSession implements WorkflowAgentSession {
  constructor(
    private readonly coordinator: WorkflowAgentCoordinator,
    readonly internal: WorkflowAgentSessionRuntime,
    private readonly runDefaults: WorkflowAgentRunOptions = {},
    private readonly state: RuntimeAgentSessionState = {
      tail: Promise.resolve(),
    },
  ) {}

  get id(): string {
    return this.internal.id;
  }

  get model(): { provider: string; model: string; active: boolean } {
    return { ...this.internal.getModel(), active: true };
  }

  run(
    prompt: string,
    options?: WorkflowAgentRunOptions,
  ): Promise<WorkflowAgentResponse> {
    return this.enqueue(() =>
      this.coordinator.run(
        this,
        prompt,
        mergeRunOptions(this.runDefaults, options),
      ),
    );
  }

  enqueue<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    const result = this.state.tail.then(operation);
    this.state.tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  withRunDefaults(
    runDefaults: WorkflowAgentRunOptions,
  ): RuntimeAgentSession {
    return new RuntimeAgentSession(
      this.coordinator,
      this.internal,
      runDefaults,
      this.state,
    );
  }
}

export class WorkflowAgentCoordinator {
  readonly api: WorkflowAgentsApi;

  constructor(
    private readonly agent: WorkflowAgentRuntime,
    private readonly signal: AbortSignal,
    private readonly onLog?: (message: string, data?: JsonValue) => void,
  ) {
    this.api = {
      create: (options) => this.create(options),
      fork: (session, options) => this.fork(session, options),
    };
  }

  async run(
    session: RuntimeAgentSession,
    prompt: string,
    options: WorkflowAgentRunOptions = {},
  ): Promise<WorkflowAgentResponse> {
    this.signal.throwIfAborted();
    this.onLog?.("Agent started.", {
      agentId: session.id,
      model: session.model.provider + "/" + session.model.model,
    });
    let content: string;
    try {
      content = await session.internal.run(
        prompt,
        {
          tools: options.tools ?? "default",
          ...(options.thinking === undefined
            ? {}
            : { thinking: options.thinking }),
        },
        this.signal,
      );
    } catch (error) {
      this.onLog?.("Agent failed.", {
        agentId: session.id,
        error: error instanceof Error ? error.message : String(error),
      });
      throw error;
    }
    this.onLog?.("Agent completed.", { agentId: session.id });
    return { content, model: session.model };
  }

  async resolveElevationSession(
    model: string,
    context: ElevationContext,
    runDefaults: WorkflowAgentRunOptions = {},
  ): Promise<WorkflowAgentSession> {
    let session: WorkflowAgentSession;
    if (context.mode === "fresh") {
      session = await this.create({ model });
    } else {
      const source = this.asRuntimeSession(context.session);
      if (context.mode === "fork") {
        session = await this.fork(source, { model });
      } else {
        await this.retarget(source, model);
        session = source;
      }
    }
    return this.asRuntimeSession(session).withRunDefaults(runDefaults);
  }

  private async create(
    options: WorkflowAgentCreateOptions,
  ): Promise<WorkflowAgentSession> {
    this.signal.throwIfAborted();
    return new RuntimeAgentSession(
      this,
      this.agent.createSession(
        options.model,
        undefined,
        undefined,
      ),
    );
  }

  private async fork(
    source: WorkflowAgentSession,
    options: WorkflowAgentForkOptions = {},
  ): Promise<WorkflowAgentSession> {
    const runtimeSource = this.asRuntimeSession(source);
    return runtimeSource.enqueue(async () => {
      this.signal.throwIfAborted();
      return new RuntimeAgentSession(
        this,
        this.agent.forkSession(
          runtimeSource.internal,
          options.model,
        ),
      );
    });
  }

  private async retarget(
    session: RuntimeAgentSession,
    modelSpec: string,
  ): Promise<void> {
    return session.enqueue(async () => {
      this.signal.throwIfAborted();
      this.agent.retargetSession(session.internal, modelSpec);
    });
  }

  private asRuntimeSession(
    session: WorkflowAgentSession,
  ): RuntimeAgentSession {
    if (!(session instanceof RuntimeAgentSession)) {
      throw new TypeError("The agent session belongs to another workflow runtime.");
    }
    return session;
  }
}
