import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { WorkflowRunStore } from "#src/workflows/WorkflowRunStore.js";

test("migrates existing workflow runs to agent and trigger metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-run-migration-"));
  await mkdir(root, { recursive: true });
  const database = new DatabaseSync(path.join(root, "runs.sqlite"));
  try {
    database.exec(`
      CREATE TABLE workflow_runs (
        id TEXT PRIMARY KEY,
        workflow_name TEXT NOT NULL,
        project_dir TEXT NOT NULL,
        source_entry_path TEXT NOT NULL,
        source_fingerprint TEXT NOT NULL,
        status TEXT NOT NULL,
        presentation TEXT NOT NULL,
        input_json TEXT NOT NULL,
        output_json TEXT,
        parent_run_id TEXT,
        depth INTEGER NOT NULL,
        error TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      INSERT INTO workflow_runs VALUES (
        'old-run', 'legacy', '/project', '/workflow.js', 'fingerprint',
        'completed', 'direct', '""', '"done"', NULL, 0, NULL,
        '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'
      );
    `);
  } finally {
    database.close();
  }

  const store = new WorkflowRunStore(root);
  try {
    const run = store.getRun("old-run");
    assert.equal(run?.agentName, "main");
    assert.deepEqual(run?.trigger, { type: "manual" });
    assert.equal(run?.output, "done");
  } finally {
    store.close();
    await rm(root, { recursive: true, force: true });
  }
});
