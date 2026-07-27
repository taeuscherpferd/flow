import type { AgentManager } from "#src/agents/AgentManager.js";
import { CronExpression, validateTimezone } from "#src/scheduling/CronExpression.js";
import { ScheduleStore } from "#src/scheduling/ScheduleStore.js";
import type {
  CreateScheduleInput,
  ScheduleOccurrence,
  ScheduleRecord,
} from "#src/scheduling/types.js";
import type { JsonValue, WorkflowRecord } from "#src/workflows/types.js";

export interface ScheduleRequest {
  agentName: string;
  workflowName: string;
  input: JsonValue;
  cron: string;
  timezone?: string;
  now?: Date;
}

export interface PreparedScheduleReauthorization {
  id: string;
  expectedUpdatedAt: string;
  packageFingerprint: string;
  nextRunAt: Date;
  confirmation: ScheduleRecord;
}

export class ScheduleService {
  constructor(
    private readonly manager: AgentManager,
    private readonly store = new ScheduleStore(manager.globalDir),
  ) {}

  create(request: ScheduleRequest): ScheduleRecord {
    const prepared = this.prepare(request);
    return this.store.create(prepared.input, prepared.nextRunAt);
  }

  previewConfirmation(request: ScheduleRequest): string {
    const prepared = this.prepare(request);
    return [
      `Agent: ${prepared.input.agentName}`,
      `Workflow: ${prepared.input.agentName}/${prepared.input.workflowName}`,
      `Input: ${JSON.stringify(prepared.input.input)}`,
      `Working directory: ${prepared.input.projectDir}`,
      `Timezone: ${prepared.input.timezone}`,
      `Cadence: ${prepared.input.cron}`,
      `Package fingerprint: ${prepared.input.packageFingerprint}`,
    ].join("\n");
  }

  private prepare(request: ScheduleRequest): {
    input: CreateScheduleInput;
    nextRunAt: Date;
  } {
    const resolved = this.resolveOwnedWorkflow(
      request.agentName,
      request.workflowName,
    );
    const registry = this.manager.getWorkflowRegistry(request.agentName)!;
    registry.validateInput(resolved, request.input);
    const timezone =
      request.timezone ??
      Intl.DateTimeFormat().resolvedOptions().timeZone;
    validateTimezone(timezone);
    const cron = CronExpression.parse(request.cron);
    const now = request.now ?? new Date();
    const input: CreateScheduleInput = {
      projectDir: this.manager.projectDir,
      agentName: request.agentName,
      workflowName: resolved.definition.name,
      input: request.input,
      cron: cron.source,
      timezone,
      packageFingerprint: this.currentFingerprint(
        request.agentName,
        resolved,
      ),
      now,
    };
    return { input, nextRunAt: cron.next(now, timezone) };
  }

  list(): ScheduleRecord[] {
    return this.store.list(this.manager.projectDir);
  }

  get(id: string): ScheduleRecord | undefined {
    const schedule = this.store.get(id);
    return schedule?.projectDir === this.manager.projectDir
      ? schedule
      : undefined;
  }

  occurrences(id: string): ScheduleOccurrence[] {
    if (!this.get(id)) throw new Error(`Unknown schedule "${id}".`);
    return this.store.listOccurrences(id);
  }

  pause(id: string): void {
    this.requireProjectSchedule(id);
    this.store.setStatus(id, "paused");
  }

  resume(id: string): void {
    const schedule = this.requireProjectSchedule(id);
    if (schedule.status === "needs-reauthorization") {
      throw new Error(
        `Schedule "${id}" needs reauthorization because its agent package changed.`,
      );
    }
    this.store.setStatus(id, "active");
  }

  delete(id: string): void {
    this.requireProjectSchedule(id);
    this.store.delete(id);
  }

  prepareReauthorization(
    id: string,
    now = new Date(),
  ): PreparedScheduleReauthorization {
    const schedule = this.requireProjectSchedule(id);
    const workflow = this.resolveOwnedWorkflow(
      schedule.agentName,
      schedule.workflowName,
    );
    this.manager
      .getWorkflowRegistry(schedule.agentName)!
      .validateInput(workflow, schedule.input);
    const next = CronExpression.parse(schedule.cron).next(
      now,
      schedule.timezone,
    );
    const packageFingerprint = this.currentFingerprint(
      schedule.agentName,
      workflow,
    );
    return {
      id,
      expectedUpdatedAt: schedule.updatedAt,
      packageFingerprint,
      nextRunAt: next,
      confirmation: {
        ...schedule,
        packageFingerprint,
        status: "active",
        nextRunAt: next.toISOString(),
      },
    };
  }

  reauthorize(
    prepared: PreparedScheduleReauthorization,
  ): ScheduleRecord {
    this.requireProjectSchedule(prepared.id);
    const updated = this.store.reauthorize(
      prepared.id,
      prepared.packageFingerprint,
      prepared.nextRunAt,
      prepared.expectedUpdatedAt,
    );
    if (!updated) {
      throw new Error(
        `Schedule "${prepared.id}" changed while reauthorization was awaiting approval. Review it again.`,
      );
    }
    return this.store.get(prepared.id)!;
  }

  confirmation(schedule: ScheduleRecord): string {
    return [
      `Agent: ${schedule.agentName}`,
      `Workflow: ${schedule.agentName}/${schedule.workflowName}`,
      `Input: ${JSON.stringify(schedule.input)}`,
      `Working directory: ${schedule.projectDir}`,
      `Timezone: ${schedule.timezone}`,
      `Cadence: ${schedule.cron}`,
      `Package fingerprint: ${schedule.packageFingerprint}`,
    ].join("\n");
  }

  close(): void {
    this.store.close();
  }

  private resolveOwnedWorkflow(
    agentName: string,
    requested: string,
  ): WorkflowRecord {
    const registry = this.manager.getWorkflowRegistry(agentName);
    if (!registry) throw new Error(`Unknown agent "${agentName}".`);
    const slash = requested.indexOf("/");
    if (slash !== -1 && requested.slice(0, slash) !== agentName) {
      throw new Error(
        `Workflow "${requested}" is not owned by agent "${agentName}".`,
      );
    }
    const name = slash === -1 ? requested : requested.slice(slash + 1);
    const record = registry.get(name);
    if (!record) {
      throw new Error(`Unknown workflow "${agentName}/${name}".`);
    }
    return record;
  }

  private currentFingerprint(
    agentName: string,
    workflow: WorkflowRecord,
  ): string {
    return (
      this.manager.getPackage(agentName)?.fingerprint.value ??
      workflow.fingerprint
    );
  }

  private requireProjectSchedule(id: string): ScheduleRecord {
    const schedule = this.get(id);
    if (!schedule) throw new Error(`Unknown schedule "${id}".`);
    return schedule;
  }
}
