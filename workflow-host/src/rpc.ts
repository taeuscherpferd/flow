import { createInterface, type Interface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import { isJsonObject } from "./json.js";
import type { JsonObject, JsonValue } from "./types.js";

export const JSON_RPC_VERSION = "2.0";

type RpcId = number | string;

interface PendingRequest {
  resolve(value: JsonValue): void;
  reject(error: Error): void;
}

export class RpcFailure extends Error {
  constructor(
    readonly code: number,
    message: string,
    readonly data?: JsonValue,
  ) {
    super(message);
  }
}

export type RpcRequestHandler = (
  method: string,
  params: JsonValue,
) => Promise<JsonValue>;

export type RpcNotificationHandler = (
  method: string,
  params: JsonValue,
) => Promise<void>;

export class RpcConnection {
  private readonly input: Interface;
  private readonly pending = new Map<RpcId, PendingRequest>();
  private nextId = 1;
  private writeTail: Promise<void> = Promise.resolve();
  private stopped = false;

  constructor(
    input: Readable,
    private readonly output: Writable,
  ) {
    this.input = createInterface({ input, crlfDelay: Infinity });
  }

  async start(
    onRequest: RpcRequestHandler,
    onNotification: RpcNotificationHandler = async () => undefined,
  ): Promise<void> {
    await this.notify("host.ready", {
      protocolVersion: 1,
      runtime: `node/${process.versions.node}`,
    });
    for await (const line of this.input) {
      if (line.trim().length === 0) continue;
      let message: JsonValue;
      try {
        message = JSON.parse(line) as JsonValue;
      } catch (error) {
        await this.writeError(
          null,
          new RpcFailure(
            -32700,
            error instanceof Error ? error.message : String(error),
          ),
        );
        continue;
      }
      void this.handleMessage(message, onRequest, onNotification);
    }
    this.stop(new Error("Rust closed the workflow host input."));
  }

  request(method: string, params: JsonValue): Promise<JsonValue> {
    if (this.stopped) {
      return Promise.reject(
        new Error("The workflow host connection is closed."),
      );
    }
    const id = this.nextId;
    this.nextId += 1;
    const response = new Promise<JsonValue>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    void this.write({
      jsonrpc: JSON_RPC_VERSION,
      id,
      method,
      params,
    }).catch((error) => {
      const failure =
        error instanceof Error ? error : new Error(String(error));
      const pending = this.pending.get(id);
      this.pending.delete(id);
      pending?.reject(failure);
    });
    return response;
  }

  notify(method: string, params: JsonValue): Promise<void> {
    return this.write({
      jsonrpc: JSON_RPC_VERSION,
      method,
      params,
    });
  }

  close(): void {
    this.input.close();
  }

  private async handleMessage(
    message: JsonValue,
    onRequest: RpcRequestHandler,
    onNotification: RpcNotificationHandler,
  ): Promise<void> {
    if (!isJsonObject(message) || message["jsonrpc"] !== JSON_RPC_VERSION) {
      await this.writeError(
        null,
        new RpcFailure(-32600, "Invalid JSON-RPC message."),
      );
      return;
    }

    const method = message["method"];
    if (typeof method === "string") {
      const params = message["params"] ?? null;
      const id = this.readId(message);
      if (id === undefined) {
        await onNotification(method, params);
        return;
      }
      try {
        const result = await onRequest(method, params);
        await this.write({
          jsonrpc: JSON_RPC_VERSION,
          id,
          result,
        });
        if (method === "host.shutdown") this.close();
      } catch (error) {
        const failure =
          error instanceof RpcFailure
            ? error
            : new RpcFailure(
                -32603,
                error instanceof Error ? error.message : String(error),
              );
        await this.writeError(id, failure);
      }
      return;
    }

    const id = this.readId(message);
    if (id === undefined) return;
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    if ("result" in message) {
      pending.resolve(message["result"] ?? null);
      return;
    }
    const errorValue = message["error"];
    if (errorValue !== undefined && isJsonObject(errorValue)) {
      const code = errorValue["code"];
      const errorMessage = errorValue["message"];
      pending.reject(
        new RpcFailure(
          typeof code === "number" ? code : -32603,
          typeof errorMessage === "string"
            ? errorMessage
            : "Rust returned an invalid JSON-RPC error.",
          errorValue["data"],
        ),
      );
      return;
    }
    pending.reject(new Error("Rust returned an invalid JSON-RPC response."));
  }

  private readId(message: JsonObject): RpcId | undefined {
    const id = message["id"];
    return typeof id === "number" || typeof id === "string"
      ? id
      : undefined;
  }

  private writeError(
    id: RpcId | null,
    failure: RpcFailure,
  ): Promise<void> {
    const error: JsonObject = {
      code: failure.code,
      message: failure.message,
      ...(failure.data === undefined ? {} : { data: failure.data }),
    };
    return this.write({
      jsonrpc: JSON_RPC_VERSION,
      id,
      error,
    });
  }

  private write(message: JsonObject): Promise<void> {
    const encoded = `${JSON.stringify(message)}\n`;
    const operation = this.writeTail.then(
      () =>
        new Promise<void>((resolve, reject) => {
          this.output.write(encoded, (error) => {
            if (error) reject(error);
            else resolve();
          });
        }),
    );
    this.writeTail = operation.catch(() => undefined);
    return operation;
  }

  private stop(error: Error): void {
    if (this.stopped) return;
    this.stopped = true;
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
  }
}
