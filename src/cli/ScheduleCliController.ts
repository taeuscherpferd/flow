import type { AgentManager } from "#src/agents/AgentManager.js";
import { ScheduleService } from "#src/scheduling/ScheduleService.js";
import { ScheduleStore } from "#src/scheduling/ScheduleStore.js";
import { ensureScheduleWorker } from "#src/scheduling/ScheduleWorkerLauncher.js";
import { createScheduleTools } from "#src/tools/schedules.js";
import type { JsonObject } from "#src/workflows/types.js";

export interface ScheduleCliUi {
  confirm(prompt: string, details: string): Promise<boolean>;
}

export class ScheduleCliController {
  private constructor(
    private readonly manager: AgentManager,
    private readonly schedules: ScheduleService,
    private readonly ui: ScheduleCliUi,
  ) {}

  static create(
    manager: AgentManager,
    ui: ScheduleCliUi,
  ): ScheduleCliController {
    const schedules = new ScheduleService(manager);
    for (const tool of createScheduleTools(manager, schedules)) {
      manager.registerDirectTool(tool);
    }
    ensureScheduleWorker(manager.globalDir);
    return new ScheduleCliController(manager, schedules, ui);
  }

  showSchedules(): void {
    const schedules = this.schedules.list();
    if (schedules.length === 0) {
      console.log("No schedules.");
      return;
    }
    for (const schedule of schedules) {
      console.log(
        `${schedule.id}  ${schedule.status.padEnd(21)}  ` +
          `${schedule.agentName}/${schedule.workflowName}  ` +
          `${schedule.cron} ${schedule.timezone}  next ${schedule.nextRunAt}`,
      );
    }
  }

  async handleCommand(command: string): Promise<void> {
    const parts = command.split(/\s+/).filter(Boolean);
    if (parts.length === 1) {
      this.inspect(parts[0]!);
      return;
    }
    const [action, id] = parts;
    if (
      !id ||
      !["pause", "resume", "delete", "reauthorize"].includes(action ?? "")
    ) {
      console.log(
        "Usage: /schedule <id> or /schedule pause|resume|delete|reauthorize <id>",
      );
      return;
    }
    try {
      if (action === "pause") this.schedules.pause(id);
      if (action === "resume") {
        this.schedules.resume(id);
        ensureScheduleWorker(this.manager.globalDir);
      }
      if (action === "delete") this.schedules.delete(id);
      if (action === "reauthorize") {
        await this.reauthorize(id);
        return;
      }
      console.log(`${action}d schedule ${id}.`);
    } catch (error) {
      console.log(error instanceof Error ? error.message : String(error));
    }
  }

  previewCreation(args: JsonObject): string | undefined {
    const agentName =
      typeof args["agent"] === "string"
        ? args["agent"]
        : this.manager.getActiveName();
    const workflowName = args["workflow"];
    const cron = args["cron"];
    const input = args["input"];
    const timezone = args["timezone"];
    if (
      typeof workflowName !== "string" ||
      typeof cron !== "string" ||
      input === undefined
    ) {
      return undefined;
    }
    return this.schedules.previewConfirmation({
      agentName,
      workflowName,
      input,
      cron,
      ...(typeof timezone === "string" ? { timezone } : {}),
    });
  }

  showUnreadEvents(): void {
    const store = new ScheduleStore(this.manager.globalDir);
    try {
      const unread = store.unread(this.manager.projectDir);
      if (unread.length === 0) return;
      const counts = new Map<string, number>();
      for (const event of unread) {
        counts.set(event.kind, (counts.get(event.kind) ?? 0) + 1);
      }
      console.log(
        `Schedule events: ${Array.from(counts, ([kind, count]) => `${count} ${kind}`).join(", ")}.`,
      );
      store.markNotificationsRead(this.manager.projectDir);
    } finally {
      store.close();
    }
  }

  close(): void {
    this.schedules.close();
  }

  private inspect(id: string): void {
    const schedule = this.schedules.get(id);
    if (!schedule) {
      console.log(`Unknown schedule "${id}".`);
      return;
    }
    console.log(this.schedules.confirmation(schedule));
    for (const occurrence of this.schedules.occurrences(schedule.id)) {
      console.log(
        `  ${occurrence.scheduledFor}  ${occurrence.status}` +
          (occurrence.runId ? `  run ${occurrence.runId}` : "") +
          (occurrence.error ? `  ${occurrence.error}` : ""),
      );
    }
  }

  private async reauthorize(id: string): Promise<void> {
    const prepared = this.schedules.prepareReauthorization(id);
    const allowed = await this.ui.confirm(
      "Reauthorize this unattended workflow schedule?",
      this.schedules.confirmation(prepared.confirmation),
    );
    if (!allowed) {
      console.log("Schedule reauthorization cancelled.");
      return;
    }
    const updated = this.schedules.reauthorize(prepared);
    console.log(this.schedules.confirmation(updated));
    ensureScheduleWorker(this.manager.globalDir);
  }
}
