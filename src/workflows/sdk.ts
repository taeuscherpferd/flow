import type {
  JsonValue,
  WorkflowDefinition,
  WorkflowOutputApi,
  WorkflowOutputValue,
  WorkflowPresentation,
} from "./types.js";

const OUTPUT_MARKER = Symbol.for("flowmation.workflow-output");

export function defineWorkflow<
  TInput = string,
  TOutput = JsonValue,
>(
  definition: WorkflowDefinition<TInput, TOutput>,
): WorkflowDefinition<TInput, TOutput> {
  return definition;
}

export type * from "./types.js";

function createOutput<TValue extends JsonValue>(
  value: TValue,
  presentation: WorkflowPresentation,
): WorkflowOutputValue<TValue> {
  return {
    kind: "workflow-output",
    presentation,
    value,
    [OUTPUT_MARKER]: true,
  } as WorkflowOutputValue<TValue>;
}

export const workflowOutputApi: WorkflowOutputApi = {
  direct: <TValue extends JsonValue>(value: TValue) =>
    createOutput(value, "direct"),
  agent: <TValue extends JsonValue>(value: TValue) =>
    createOutput(value, "agent"),
};

export function isWorkflowOutput(
  result: JsonValue | WorkflowOutputValue,
): result is WorkflowOutputValue {
  return (
    typeof result === "object" &&
    result !== null &&
    !Array.isArray(result) &&
    Reflect.get(result, OUTPUT_MARKER) === true &&
    result.kind === "workflow-output" &&
    (result.presentation === "direct" || result.presentation === "agent") &&
    "value" in result
  );
}
