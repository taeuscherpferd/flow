import { CallbackRegistry, createWorkflowContext } from "./context.js";
import { assertJsonValue, isJsonObject } from "./json.js";
import { loadWorkflow, type LoadedWorkflow } from "./loader.js";
import {
  RpcFailure,
  type RpcConnection,
  type RpcRequestHandler,
} from "./rpc.js";
import { isWorkflowOutput } from "./sdk.js";
import type {
  AgentInvocationPolicy,
  JsonObject,
  JsonValue,
  WorkflowPresentation,
} from "./types.js";

export const PROTOCOL_VERSION = 1;

export class WorkflowHostServer {
  private readonly callbacks = new CallbackRegistry();
  private readonly activeRuns = new Map<string, AbortController>();
  private readonly workflows = new Map<string, LoadedWorkflow>();
  private handshaken = false;
  private shuttingDown = false;

  constructor(private readonly connection: RpcConnection) {}

  readonly handleRequest: RpcRequestHandler = async (method, params) => {
    if (method === "host.handshake") return this.handshake(params);
    if (!this.handshaken) {
      throw new RpcFailure(
        -32600,
        "The protocol handshake must complete before other requests.",
      );
    }

    switch (method) {
      case "workflow.inspect":
        return this.inspect(params);
      case "workflow.run":
        return this.run(params);
      case "workflow.cancel":
        return this.cancel(params);
      case "callback.invoke":
        return this.invokeCallback(params);
      case "host.shutdown":
        return this.shutdown();
      default:
        throw new RpcFailure(-32601, `Unknown host method "${method}".`);
    }
  };

  private handshake(params: JsonValue): JsonValue {
    const values = requireObject(params, "Handshake params");
    const requestedVersion = requireNumber(
      values["protocolVersion"],
      "Handshake protocolVersion",
    );
    if (requestedVersion !== PROTOCOL_VERSION) {
      throw new RpcFailure(
        -32001,
        `Unsupported workflow protocol version ${requestedVersion}; this host requires ${PROTOCOL_VERSION}.`,
        {
          requestedVersion,
          supportedVersion: PROTOCOL_VERSION,
        },
      );
    }
    this.handshaken = true;
    return {
      protocolVersion: PROTOCOL_VERSION,
      hostName: "flowmation-workflow-host",
      hostVersion: "1.0.0",
      runtime: `node/${process.versions.node}`,
      capabilities: [
        "bidirectional-callbacks",
        "cancellation",
        "javascript",
        "typescript",
      ],
    };
  }

  private async inspect(params: JsonValue): Promise<JsonValue> {
    this.assertAcceptingWork();
    const values = requireObject(params, "Inspect params");
    const workflow = await this.getWorkflow(
      requireString(values["entryPath"], "Inspect entryPath"),
    );
    const { definition } = workflow;
    return {
      metadata: {
        name: definition.name,
        description: definition.description,
        ...(definition.input === undefined
          ? {}
          : { inputSchema: definition.input.schema as JsonValue }),
        agentInvocation:
          (definition.agentInvocation ??
            "disabled") satisfies AgentInvocationPolicy,
        presentation:
          (definition.presentation ??
            "direct") satisfies WorkflowPresentation,
      },
    };
  }

  private async run(params: JsonValue): Promise<JsonValue> {
    this.assertAcceptingWork();
    const values = requireObject(params, "Run params");
    const runId = requireString(values["runId"], "Run runId");
    if (this.activeRuns.has(runId)) {
      throw new RpcFailure(
        -32602,
        `Workflow run "${runId}" is already active in this host.`,
      );
    }
    const entryPath = requireString(
      values["entryPath"],
      "Run entryPath",
    );
    const projectDir = requireString(
      values["projectDir"],
      "Run projectDir",
    );
    const input = values["input"] ?? null;
    assertJsonValue(input, "Workflow input");
    const workflow = await this.getWorkflow(entryPath);
    const controller = new AbortController();
    this.activeRuns.set(runId, controller);
    await this.connection.notify("workflow.event", {
      runId,
      kind: "started",
    });

    try {
      const context = createWorkflowContext({
        runId,
        projectDir,
        signal: controller.signal,
        connection: this.connection,
        callbacks: this.callbacks,
      });
      const result = await workflow.definition.run(context, input);
      controller.signal.throwIfAborted();
      const presentation = isWorkflowOutput(result)
        ? result.presentation
        : (workflow.definition.presentation ?? "direct");
      const value = isWorkflowOutput(result) ? result.value : result;
      assertJsonValue(value, "Workflow result");
      await this.connection.notify("workflow.event", {
        runId,
        kind: "completed",
      });
      return { value, presentation };
    } catch (error) {
      const cancelled = controller.signal.aborted;
      await this.connection.notify("workflow.event", {
        runId,
        kind: cancelled ? "cancelled" : "failed",
        ...(!cancelled && error instanceof Error
          ? { data: { message: error.message } }
          : {}),
      });
      if (cancelled) {
        throw new RpcFailure(
          -32002,
          controller.signal.reason instanceof Error
            ? controller.signal.reason.message
            : `Workflow run "${runId}" was cancelled.`,
        );
      }
      throw error;
    } finally {
      this.activeRuns.delete(runId);
    }
  }

  private cancel(params: JsonValue): JsonValue {
    const values = requireObject(params, "Cancel params");
    const runId = requireString(values["runId"], "Cancel runId");
    const controller = this.activeRuns.get(runId);
    if (!controller) return { cancelled: false };
    const reason =
      typeof values["reason"] === "string"
        ? values["reason"]
        : `Workflow run "${runId}" was cancelled.`;
    controller.abort(new Error(reason));
    return { cancelled: true };
  }

  private async invokeCallback(params: JsonValue): Promise<JsonValue> {
    const values = requireObject(params, "Callback params");
    const callbackId = requireString(
      values["callbackId"],
      "Callback callbackId",
    );
    const argumentsValue = values["arguments"];
    if (!Array.isArray(argumentsValue)) {
      throw new RpcFailure(
        -32602,
        "Callback arguments must be an array.",
      );
    }
    return this.callbacks.invoke(callbackId, argumentsValue);
  }

  private shutdown(): JsonValue {
    this.shuttingDown = true;
    for (const [runId, controller] of this.activeRuns) {
      controller.abort(
        new Error(`Workflow run "${runId}" was interrupted by shutdown.`),
      );
    }
    return null;
  }

  private async getWorkflow(entryPath: string): Promise<LoadedWorkflow> {
    const cached = this.workflows.get(entryPath);
    if (cached) return cached;
    const workflow = await loadWorkflow(entryPath);
    this.workflows.set(entryPath, workflow);
    return workflow;
  }

  private assertAcceptingWork(): void {
    if (this.shuttingDown) {
      throw new RpcFailure(-32600, "The workflow host is shutting down.");
    }
  }
}

function requireObject(value: JsonValue, label: string): JsonObject {
  if (!isJsonObject(value)) {
    throw new RpcFailure(-32602, `${label} must be an object.`);
  }
  return value;
}

function requireString(
  value: JsonValue | undefined,
  label: string,
): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new RpcFailure(-32602, `${label} must be a non-empty string.`);
  }
  return value;
}

function requireNumber(
  value: JsonValue | undefined,
  label: string,
): number {
  if (typeof value !== "number" || !Number.isInteger(value)) {
    throw new RpcFailure(-32602, `${label} must be an integer.`);
  }
  return value;
}
