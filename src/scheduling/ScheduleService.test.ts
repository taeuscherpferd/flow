import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AgentManager } from "#src/agents/AgentManager.js";
import { ConfigService } from "#src/services/ConfigService.js";
import { ScheduleService } from "#src/scheduling/ScheduleService.js";

test("reauthorizes the exact prospective fingerprint shown for approval", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-reauthorize-"));
  const globalDir = path.join(root, "global");
  const projectDir = path.join(root, "project", ".work-agent");
  const workflowDir = path.join(projectDir, "workflows", "scheduled-report");
  const workflowPath = path.join(workflowDir, "WORKFLOW.js");
  const config = new ConfigService({ globalDir, projectDir });
  let manager: AgentManager | undefined;
  let schedules: ScheduleService | undefined;
  try {
    await config.saveModelSetup({
      provider: "local",
      baseUrl: "http://localhost:11434",
      model: "test-model",
      contextWindow: 8192,
    });
    await mkdir(workflowDir, { recursive: true });
    await writeFile(
      workflowPath,
      `export default {
        name: "scheduled-report",
        description: "Produces a report",
        async run() { return "first"; }
      };`,
    );
    manager = await AgentManager.create(config);
    schedules = new ScheduleService(manager);
    const schedule = schedules.create({
      agentName: "main",
      workflowName: "scheduled-report",
      input: "",
      cron: "* * * * *",
      timezone: "UTC",
      now: new Date("2026-07-25T12:00:00.000Z"),
    });
    schedules.close();
    schedules = undefined;
    manager.close();
    manager = undefined;

    await writeFile(
      workflowPath,
      `export default {
        name: "scheduled-report",
        description: "Produces a changed report",
        async run() { return "second"; }
      };`,
    );
    manager = await AgentManager.create(config);
    schedules = new ScheduleService(manager);
    const prepared = schedules.prepareReauthorization(
      schedule.id,
      new Date("2026-07-25T12:05:00.000Z"),
    );

    assert.notEqual(
      prepared.confirmation.packageFingerprint,
      schedule.packageFingerprint,
    );
    assert.equal(
      schedules.get(schedule.id)?.packageFingerprint,
      schedule.packageFingerprint,
    );
    const updated = schedules.reauthorize(prepared);
    assert.equal(
      updated.packageFingerprint,
      prepared.confirmation.packageFingerprint,
    );
    assert.equal(updated.nextRunAt, prepared.confirmation.nextRunAt);
  } finally {
    schedules?.close();
    manager?.close();
    await rm(root, { recursive: true, force: true });
  }
});
