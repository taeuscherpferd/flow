import type { Tool } from "#src/tools/types.js";
import type {
  AgentInvocationPolicy,
  JsonValue,
  WorkflowRecord,
  WorkflowSchema,
} from "#src/workflows/types.js";

export interface WorkflowToolRuntime {
  resolve(name: string): Promise<WorkflowRecord | undefined>;
  invoke(
    record: WorkflowRecord,
    input: JsonValue,
    signal?: AbortSignal,
  ): Promise<string>;
  confirm(record: WorkflowRecord, input: JsonValue): Promise<boolean>;
}

function formatWorkflowSchema(schema: WorkflowSchema): string {
  return JSON.stringify(schema);
}

export function createRunWorkflowTool(
  workflows: WorkflowRecord[],
  runtime: WorkflowToolRuntime,
): Tool {
  const eligible = workflows.filter(
    (record) =>
      (record.definition.agentInvocation ?? "disabled") !== "disabled",
  );
  const structuredInputContracts = eligible
    .flatMap((record) => {
      const schema = record.definition.input?.schema;
      return schema?.type === "object"
        ? [`${record.definition.name}: ${formatWorkflowSchema(schema)}`]
        : [];
    })
    .join("\n");

  return {
    name: "run_workflow",
    description:
      "Run an eligible developer workflow when it directly matches the user's request. " +
      "Use inputText for string workflows and input for schema-based object workflows.",
    parameters: {
      type: "object",
      properties: {
        name: {
          type: "string",
          description: "The workflow name.",
          enum: eligible.map((record) => record.definition.name),
        },
        inputText: {
          type: "string",
          description: "Plain text input for a string workflow.",
        },
        input: {
          type: "object",
          description:
            "Structured input for an object-schema workflow." +
            (structuredInputContracts.length === 0
              ? ""
              : ` Match the selected workflow's schema:\n${structuredInputContracts}`),
        },
      },
      required: ["name"],
    },
    async execute(args, _context, signal) {
      signal?.throwIfAborted();
      const name = args["name"];
      if (typeof name !== "string") {
        return { ok: false, content: "Error: workflow name must be a string." };
      }
      const record = await runtime.resolve(name);
      if (
        !record ||
        (record.definition.agentInvocation ?? "disabled") === "disabled"
      ) {
        return { ok: false, content: `Error: workflow "${name}" is not eligible.` };
      }

      let input: JsonValue;
      const schema = record.definition.input?.schema;
      if (!schema || schema.type === "string") {
        const inputText = args["inputText"];
        if (typeof inputText !== "string") {
          return {
            ok: false,
            content: `Error: workflow "${name}" requires inputText.`,
          };
        }
        input = inputText;
      } else {
        const structured = args["input"];
        if (
          typeof structured !== "object" ||
          structured === null ||
          Array.isArray(structured)
        ) {
          return {
            ok: false,
            content: `Error: workflow "${name}" requires object input.`,
          };
        }
        input = structured;
      }

      const policy: AgentInvocationPolicy =
        record.definition.agentInvocation ?? "disabled";
      if (policy === "confirm" && !(await runtime.confirm(record, input))) {
        return {
          ok: false,
          content: `The user declined workflow "${name}".`,
        };
      }

      try {
        signal?.throwIfAborted();
        return {
          ok: true,
          content: await runtime.invoke(record, input, signal),
        };
      } catch (error) {
        if (signal?.aborted) throw error;
        return {
          ok: false,
          content: `Error running workflow "${name}": ${
            error instanceof Error ? error.message : String(error)
          }`,
        };
      }
    },
  };
}

export function buildWorkflowSystemContext(
  workflows: WorkflowRecord[],
): string {
  const eligible = workflows.filter(
    (record) =>
      (record.definition.agentInvocation ?? "disabled") !== "disabled",
  );
  if (eligible.length === 0) return "";

  const listing = eligible
    .map((record) => {
      const policy = record.definition.agentInvocation ?? "disabled";
      const input =
        record.definition.input?.schema.type === "object"
          ? `structured input matching ${formatWorkflowSchema(
              record.definition.input.schema,
            )}`
          : "plain text input";
      return `- ${record.definition.name} (${policy}, ${input}): ${record.definition.description}`;
    })
    .join("\n");
  return (
    "## Available Workflows\n\n" +
    "Use run_workflow only when one of these workflows directly matches the request.\n\n" +
    listing
  );
}
