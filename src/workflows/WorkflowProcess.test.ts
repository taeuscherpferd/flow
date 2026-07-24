import assert from "node:assert/strict";
import { access, mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { runWorkflowCommand } from "./WorkflowProcess.js";

async function pathExists(filePath: string): Promise<boolean> {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function waitForPath(filePath: string): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (await pathExists(filePath)) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`Timed out waiting for "${filePath}".`);
}

test("cancellation stops an active command", { timeout: 5_000 }, async () => {
  const controller = new AbortController();
  const execution = runWorkflowCommand(
    process.cwd(),
    controller.signal,
    process.execPath,
    ["-e", "setInterval(() => {}, 1000)"],
  );
  await new Promise((resolve) => setTimeout(resolve, 50));
  controller.abort();

  await assert.rejects(execution, /cancelled/);
});

test("cancellation terminates descendant processes", {
  timeout: 5_000,
  skip:
    process.platform === "win32"
      ? "The managed Windows test environment blocks taskkill /T."
      : false,
}, async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-process-"));
  const startedPath = path.join(root, "started");
  const descendantPath = path.join(root, "descendant");
  const controller = new AbortController();
  const parentScript = `
    const { spawn } = require("node:child_process");
    const { writeFileSync } = require("node:fs");
    writeFileSync(${JSON.stringify(startedPath)}, "started");
    const child = spawn(process.execPath, [
      "-e",
      ${JSON.stringify(
        `setTimeout(() => require("node:fs").writeFileSync(${JSON.stringify(
          descendantPath,
        )}, "alive"), 1200)`,
      )}
    ], { stdio: "ignore" });
    setInterval(() => {}, 1000);
  `;

  try {
    const execution = runWorkflowCommand(
      root,
      controller.signal,
      process.execPath,
      ["-e", parentScript],
    );
    await waitForPath(startedPath);
    controller.abort();

    await assert.rejects(execution, /cancelled/);
    await new Promise((resolve) => setTimeout(resolve, 1500));
    assert.equal(await pathExists(descendantPath), false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
