import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ScheduleStore } from "#src/scheduling/ScheduleStore.js";

test("stores schedules, unique occurrences, and renewable worker leases", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-schedules-"));
  const store = new ScheduleStore(root);
  try {
    const now = new Date("2026-01-01T00:00:00.000Z");
    const schedule = store.create(
      {
        projectDir: "/project",
        agentName: "finance",
        workflowName: "monthly-close",
        input: { month: "2025-12" },
        cron: "0 9 1 * *",
        timezone: "America/Denver",
        packageFingerprint: "abc123",
        now,
      },
      new Date("2026-02-01T16:00:00.000Z"),
    );
    const occurrence = store.claimDueOccurrence(
      schedule.id,
      "2026-02-01T16:00:00.000Z",
      new Date("2026-03-01T16:00:00.000Z"),
    );

    assert.ok(occurrence);
    assert.equal(
      store.createOccurrence(
        schedule.id,
        "2026-02-01T16:00:00.000Z",
      ),
      undefined,
    );
    assert.equal(
      store.claimDueOccurrence(
        schedule.id,
        "2026-03-01T16:00:00.000Z",
        new Date("2026-04-01T15:00:00.000Z"),
      ),
      undefined,
    );
    assert.equal(store.get(schedule.id)?.nextRunAt, "2026-04-01T15:00:00.000Z");
    assert.deepEqual(
      store
        .listOccurrences(schedule.id)
        .map((entry) => entry.status)
        .sort(),
      ["pending", "skipped"],
    );
    assert.equal(store.acquireLease("worker", "one", now, 30_000), true);
    assert.equal(store.acquireLease("worker", "two", now, 30_000), false);
    assert.equal(
      store.acquireLease(
        "worker",
        "two",
        new Date(now.getTime() + 31_000),
        30_000,
      ),
      true,
    );
  } finally {
    store.close();
    await rm(root, { recursive: true, force: true });
  }
});
