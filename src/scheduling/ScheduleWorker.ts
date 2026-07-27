import { randomUUID } from "node:crypto";
import path from "node:path";
import { AgentManager } from "#src/agents/AgentManager.js";
import { ConfigService } from "#src/services/ConfigService.js";
import { CronExpression } from "#src/scheduling/CronExpression.js";
import { ScheduleStore } from "#src/scheduling/ScheduleStore.js";
import type {
  ScheduleOccurrence,
  ScheduleOccurrenceStatus,
  ScheduleRecord,
} from "#src/scheduling/types.js";
import type { WorkflowRunStatus } from "#src/workflows/types.js";
import { WorkflowEngine } from "#src/workflows/WorkflowEngine.js";
import { WorkflowRunStore } from "#src/workflows/WorkflowRunStore.js";

const LEASE_KEY = "schedule-worker";
const LEASE_MS = 45_000;

function occurrenceStatus(status: WorkflowRunStatus): ScheduleOccurrenceStatus {
  if (status === "completed" || status === "failed" || status === "waiting") {
    return status;
  }
  if (status === "queued" || status === "running") return "running";
  return "failed";
}

export class ScheduleWorker {
  private readonly ownerId = randomUUID();

  constructor(
    private readonly globalDir: string,
    private readonly store = new ScheduleStore(globalDir),
  ) {}

  async tick(now = new Date()): Promise<boolean> {
    if (!this.store.acquireLease(LEASE_KEY, this.ownerId, now, LEASE_MS)) {
      return false;
    }
    const heartbeat = setInterval(() => {
      try {
        this.store.acquireLease(
          LEASE_KEY,
          this.ownerId,
          new Date(),
          LEASE_MS,
        );
      } catch {
        // The active execution will surface its own database failure.
      }
    }, LEASE_MS / 3);
    heartbeat.unref();
    try {
      for (const occurrence of this.store.listRecoverableOccurrences()) {
        const schedule = this.store.get(occurrence.scheduleId);
        if (!schedule) continue;
        await this.execute(schedule, occurrence);
      }
      for (const schedule of this.store.listDue(now)) {
        const scheduledFor = schedule.nextRunAt;
        const next = CronExpression.parse(schedule.cron).next(
          now,
          schedule.timezone,
        );
        const occurrence = this.store.claimDueOccurrence(
          schedule.id,
          scheduledFor,
          next,
        );
        if (occurrence) await this.execute(schedule, occurrence);
      }
      return true;
    } finally {
      clearInterval(heartbeat);
    }
  }

  close(): void {
    this.store.close();
  }

  private async execute(
    schedule: ScheduleRecord,
    occurrence: ScheduleOccurrence,
  ): Promise<void> {
    const config = new ConfigService({
      globalDir: this.globalDir,
      projectDir: path.join(schedule.projectDir, ".work-agent"),
    });
    let manager: AgentManager | undefined;
    let runStore: WorkflowRunStore | undefined;
    try {
      manager = await AgentManager.createExecution(
        config,
        {
          requestPermission: async () => false,
        },
        {
          agentName: schedule.agentName,
          workflowName: schedule.workflowName,
          packageFingerprint: schedule.packageFingerprint,
        },
      );
      const registry = manager.getWorkflowRegistry(schedule.agentName);
      const workflow = registry?.get(schedule.workflowName);
      if (!registry || !workflow) {
        this.invalidate(
          schedule,
          occurrence,
          "The scheduled agent or workflow no longer exists, or its source changed.",
        );
        return;
      }
      const fingerprint =
        manager.getPackage(schedule.agentName)?.fingerprint.value ??
        workflow.fingerprint;
      if (fingerprint !== schedule.packageFingerprint) {
        this.invalidate(
          schedule,
          occurrence,
          "The agent package changed and the schedule needs reauthorization.",
        );
        return;
      }
      const agent = manager.createExecutionAgent(
        schedule.agentName,
        "scheduled",
      );
      runStore = new WorkflowRunStore(this.globalDir);
      const engine = new WorkflowEngine(
        agent,
        registry,
        runStore,
        schedule.projectDir,
        schedule.agentName,
      );
      const runId = occurrence.runId ?? randomUUID();
      this.store.updateOccurrence(occurrence.id, "running", {
        runId,
      });
      const existingRun = runStore.getRun(runId);
      const result =
        existingRun === undefined
          ? await engine.start(schedule.workflowName, schedule.input, {
              runId,
              expectedSourceFingerprint: workflow.fingerprint,
              agentName: schedule.agentName,
              trigger: {
                type: "schedule",
                scheduleId: schedule.id,
                scheduledFor: occurrence.scheduledFor,
              },
            })
          : await engine.resume(runId);
      this.store.updateOccurrence(occurrence.id, occurrenceStatus(result.run.status), {
        runId: result.run.id,
        ...(result.value === undefined ? {} : { result: result.value }),
        ...(result.run.error === undefined ? {} : { error: result.run.error }),
      });
      await engine.shutdown();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.store.updateOccurrence(occurrence.id, "failed", { error: message });
      this.store.notify(
        schedule,
        "failed",
        `Scheduled workflow ${schedule.agentName}/${schedule.workflowName} failed: ${message}`,
        occurrence.id,
      );
    } finally {
      runStore?.close();
      manager?.close();
    }
  }

  private invalidate(
    schedule: ScheduleRecord,
    occurrence: ScheduleOccurrence,
    message: string,
  ): void {
    this.store.setStatus(schedule.id, "needs-reauthorization");
    this.store.updateOccurrence(occurrence.id, "invalidated", {
      error: message,
    });
    this.store.notify(
      schedule,
      "invalidated",
      `Schedule ${schedule.id} was invalidated: ${message}`,
      occurrence.id,
    );
  }
}
