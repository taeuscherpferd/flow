import { isJsonObject } from "./json.js";
import type {
  HumanChoice,
  JsonObject,
  JsonValue,
  ModelRef,
  WorkflowAgentRunOptions,
  WorkflowCheckDetails,
  WorkflowExecOptions,
  WorkflowExecResult,
} from "./types.js";

export function compactOptions(
  options: WorkflowAgentRunOptions | WorkflowExecOptions,
): JsonObject {
  return Object.fromEntries(
    Object.entries(options).filter((entry) => entry[1] !== undefined),
  ) as JsonObject;
}

export function choiceToJson(choice: HumanChoice): JsonObject {
  return {
    value: choice.value,
    label: choice.label,
    ...(choice.description === undefined
      ? {}
      : { description: choice.description }),
  };
}

export function requireObject(
  value: JsonValue,
  label: string,
): JsonObject {
  if (!isJsonObject(value)) {
    throw new TypeError(`${label} must be an object.`);
  }
  return value;
}

export function requireString(
  value: JsonValue | undefined,
  label: string,
): string {
  if (typeof value !== "string") {
    throw new TypeError(`${label} must be a string.`);
  }
  return value;
}

export function readModel(value: JsonValue | undefined): ModelRef {
  const model = requireObject(value ?? null, "Agent model");
  const active = model["active"];
  if (typeof active !== "boolean") {
    throw new TypeError("Agent model active must be a boolean.");
  }
  return {
    provider: requireString(model["provider"], "Agent model provider"),
    model: requireString(model["model"], "Agent model name"),
    active,
  };
}

export function readExecResult(value: JsonValue): WorkflowExecResult {
  const result = requireObject(value, "Exec result");
  const args = result["args"];
  const exitCode = result["exitCode"];
  if (!Array.isArray(args) || !args.every((arg) => typeof arg === "string")) {
    throw new TypeError("Exec result args must be strings.");
  }
  if (typeof exitCode !== "number") {
    throw new TypeError("Exec result exitCode must be a number.");
  }
  return {
    command: requireString(result["command"], "Exec result command"),
    args,
    stdout: requireString(result["stdout"], "Exec result stdout"),
    stderr: requireString(result["stderr"], "Exec result stderr"),
    exitCode,
  };
}

export function readCheckDetails(
  values: JsonValue[],
): WorkflowCheckDetails[] {
  return values.map((value) => {
    const check = requireObject(value, "Elevation check");
    const passed = check["passed"];
    const message = check["message"];
    if (typeof passed !== "boolean") {
      throw new TypeError("Elevation check passed must be a boolean.");
    }
    if (message !== undefined && typeof message !== "string") {
      throw new TypeError("Elevation check message must be a string.");
    }
    return {
      passed,
      ...(message === undefined ? {} : { message }),
      ...(check["data"] === undefined ? {} : { data: check["data"] }),
    };
  });
}

export function assertPositiveInteger(
  value: number,
  label: string,
): void {
  if (!Number.isInteger(value) || value < 1) {
    throw new TypeError(`${label} must be a positive integer.`);
  }
}
