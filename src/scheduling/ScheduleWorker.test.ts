import assert from "node:assert/strict";
import {
  access,
  mkdir,
  mkdtemp,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AgentManager } from "#src/agents/AgentManager.js";
import { ConfigService } from "#src/services/ConfigService.js";
import { ScheduleService } from "#src/scheduling/ScheduleService.js";
import { ScheduleStore } from "#src/scheduling/ScheduleStore.js";
import { ScheduleWorker } from "#src/scheduling/ScheduleWorker.js";
import { fingerprintDirectory } from "#src/services/DirectoryFingerprint.js";

test("runs one catch-up occurrence and records its scheduled trigger", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-worker-"));
  const globalDir = path.join(root, "global");
  const projectRoot = path.join(root, "project");
  const projectDir = path.join(projectRoot, ".work-agent");
  const workflowDir = path.join(projectDir, "workflows", "scheduled-report");
  const config = new ConfigService({ globalDir, projectDir });
  let manager: AgentManager | undefined;
  let schedules: ScheduleService | undefined;
  let worker: ScheduleWorker | undefined;
  try {
    await config.saveModelSetup({
      provider: "local",
      baseUrl: "http://localhost:11434",
      model: "test-model",
      contextWindow: 8192,
    });
    await mkdir(workflowDir, { recursive: true });
    await writeFile(
      path.join(workflowDir, "WORKFLOW.js"),
      `export default {
        name: "scheduled-report",
        description: "Produces a scheduled report",
        async run() { return "done"; }
      };`,
    );
    manager = await AgentManager.create(config);
    schedules = new ScheduleService(manager);
    const now = new Date("2026-07-25T12:00:00.000Z");
    const schedule = schedules.create({
      agentName: "main",
      workflowName: "scheduled-report",
      input: "",
      cron: "* * * * *",
      timezone: "UTC",
      now,
    });
    schedules.close();
    schedules = undefined;
    manager.close();
    manager = undefined;

    worker = new ScheduleWorker(globalDir);
    await worker.tick(new Date("2026-07-25T12:10:00.000Z"));

    let store = new ScheduleStore(globalDir);
    try {
      const occurrences = store.listOccurrences(schedule.id);
      assert.equal(occurrences.length, 1);
      assert.equal(occurrences[0]?.scheduledFor, "2026-07-25T12:01:00.000Z");
      assert.equal(occurrences[0]?.status, "completed");
      assert.ok(occurrences[0]?.runId);
      assert.equal(store.unread(projectRoot)[0]?.kind, "completed");
      const recovering = store.createOccurrence(
        schedule.id,
        "2026-07-25T12:10:30.000Z",
      );
      assert.ok(recovering);
      store.updateOccurrence(recovering.id, "running", {
        runId: "crash-window-run",
      });
    } finally {
      store.close();
    }

    await worker.tick(new Date("2026-07-25T12:10:30.000Z"));
    worker.close();
    worker = undefined;
    store = new ScheduleStore(globalDir);
    try {
      const recovered = store
        .listOccurrences(schedule.id)
        .find((occurrence) => occurrence.runId === "crash-window-run");
      assert.equal(recovered?.status, "completed");
    } finally {
      store.close();
    }
  } finally {
    worker?.close();
    schedules?.close();
    manager?.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects changed workflow source before evaluating its module", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-worker-auth-"));
  const globalDir = path.join(root, "global");
  const projectRoot = path.join(root, "project");
  const projectDir = path.join(projectRoot, ".work-agent");
  const workflowDir = path.join(projectDir, "workflows", "scheduled-report");
  const workflowPath = path.join(workflowDir, "WORKFLOW.js");
  const sentinelPath = path.join(root, "unauthorized-module-ran");
  const config = new ConfigService({ globalDir, projectDir });
  let worker: ScheduleWorker | undefined;
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
        description: "Produces a scheduled report",
        async run() { return "safe"; }
      };`,
    );
    const store = new ScheduleStore(globalDir);
    const schedule = store.create(
      {
        projectDir: projectRoot,
        agentName: "main",
        workflowName: "scheduled-report",
        input: "",
        cron: "* * * * *",
        timezone: "UTC",
        packageFingerprint: await fingerprintDirectory(workflowDir),
        now: new Date("2026-07-25T12:00:00.000Z"),
      },
      new Date("2026-07-25T12:01:00.000Z"),
    );
    store.close();
    await writeFile(
      workflowPath,
      `import { writeFileSync } from "node:fs";
      writeFileSync(${JSON.stringify(sentinelPath)}, "executed");
      export default {
        name: "scheduled-report",
        description: "Produces a changed report",
        async run() { return "unsafe"; }
      };`,
    );

    worker = new ScheduleWorker(globalDir);
    await worker.tick(new Date("2026-07-25T12:02:00.000Z"));

    await assert.rejects(access(sentinelPath));
    const verification = new ScheduleStore(globalDir);
    try {
      assert.equal(
        verification.listOccurrences(schedule.id)[0]?.status,
        "invalidated",
      );
      assert.equal(
        verification.get(schedule.id)?.status,
        "needs-reauthorization",
      );
    } finally {
      verification.close();
    }
  } finally {
    worker?.close();
    await rm(root, { recursive: true, force: true });
  }
});
