import type { AgentManager } from "#src/agents/AgentManager.js";
import type { ScheduleService } from "#src/scheduling/ScheduleService.js";
import { ensureScheduleWorker } from "#src/scheduling/ScheduleWorkerLauncher.js";
import type { Tool } from "#src/tools/types.js";
import type { JsonValue } from "#src/workflows/types.js";

function idArgument(args: Record<string, JsonValue>): string | undefined {
  const id = args["id"];
  return typeof id === "string" && id.length > 0 ? id : undefined;
}

export function createScheduleTools(
  manager: AgentManager,
  schedules: ScheduleService,
): Tool[] {
  const create: Tool = {
    name: "create_schedule",
    effect: "schedule",
    description:
      "Create an unattended schedule for an agent-owned workflow using a five-field cron expression.",
    parameters: {
      type: "object",
      properties: {
        agent: { type: "string", description: "Owning agent name." },
        workflow: { type: "string", description: "Owned workflow name." },
        input: {
          type: ["string", "number", "boolean", "object", "array", "null"],
          description: "Workflow JSON input.",
        },
        cron: { type: "string", description: "Five-field cron expression." },
        timezone: { type: "string", description: "IANA timezone." },
      },
      required: ["workflow", "input", "cron"],
    },
    async execute(args) {
      const agent =
        typeof args["agent"] === "string"
          ? args["agent"]
          : manager.getActiveName();
      const workflow = args["workflow"];
      const cron = args["cron"];
      const input = args["input"];
      const timezone = args["timezone"];
      if (
        typeof workflow !== "string" ||
        typeof cron !== "string" ||
        input === undefined
      ) {
        return {
          ok: false,
          content: "Error: workflow, input, and cron are required.",
        };
      }
      try {
        const schedule = schedules.create({
          agentName: agent,
          workflowName: workflow,
          input,
          cron,
          ...(typeof timezone === "string" ? { timezone } : {}),
        });
        ensureScheduleWorker(manager.globalDir);
        return { ok: true, content: schedules.confirmation(schedule) };
      } catch (error) {
        return {
          ok: false,
          content: error instanceof Error ? error.message : String(error),
        };
      }
    },
  };
  const list: Tool = {
    name: "list_schedules",
    effect: "read",
    description: "List schedules for the current project.",
    parameters: { type: "object", properties: {} },
    async execute() {
      const records = schedules.list();
      return {
        ok: true,
        content:
          records.length === 0
            ? "No schedules."
            : records
                .map(
                  (record) =>
                    `${record.id} ${record.status} ${record.agentName}/${record.workflowName} ${record.cron} ${record.timezone}`,
                )
                .join("\n"),
      };
    },
  };
  const mutation = (
    name: "pause_schedule" | "resume_schedule" | "delete_schedule",
    action: (id: string) => void,
  ): Tool => ({
    name,
    effect: "schedule",
    description: `${name.split("_")[0]} a workflow schedule.`,
    parameters: {
      type: "object",
      properties: { id: { type: "string", description: "Schedule id." } },
      required: ["id"],
    },
    async execute(args) {
      const id = idArgument(args);
      if (!id) return { ok: false, content: "Error: id is required." };
      try {
        action(id);
        return { ok: true, content: `${name.split("_")[0]}d schedule ${id}.` };
      } catch (error) {
        return {
          ok: false,
          content: error instanceof Error ? error.message : String(error),
        };
      }
    },
  });
  return [
    create,
    list,
    mutation("pause_schedule", (id) => schedules.pause(id)),
    mutation("resume_schedule", (id) => {
      schedules.resume(id);
      ensureScheduleWorker(manager.globalDir);
    }),
    mutation("delete_schedule", (id) => schedules.delete(id)),
  ];
}
