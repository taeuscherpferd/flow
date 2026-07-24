import type { ChatMessage, ModelProvider } from "../providers/types.js";
import type {
  ElevationContext,
  JsonValue,
  WorkflowAgentCreateOptions,
  WorkflowAgentForkOptions,
  WorkflowAgentResponse,
  WorkflowAgentRunOptions,
  WorkflowAgentSession,
  WorkflowAgentsApi,
} from "./types.js";

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
  private tail: Promise<void> = Promise.resolve();

  constructor(
    private readonly coordinator: WorkflowAgentCoordinator,
    readonly internal: WorkflowAgentSessionRuntime,
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
    return this.enqueue(() => this.coordinator.run(this, prompt, options));
  }

  enqueue<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    const result = this.tail.then(operation);
    this.tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
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
        { tools: options.tools ?? "default" },
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
  ): Promise<WorkflowAgentSession> {
    if (context.mode === "fresh") return this.create({ model });
    const source = this.asRuntimeSession(context.session);
    if (context.mode === "fork") return this.fork(source, { model });
    await this.retarget(source, model);
    return source;
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
