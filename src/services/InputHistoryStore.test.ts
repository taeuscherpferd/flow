import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  InputHistoryStore,
  InputHistoryStoreError,
} from "#src/services/InputHistoryStore.js";

test("returns empty history when no persisted file exists", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-history-"));

  try {
    const store = new InputHistoryStore(rootDir);

    assert.deepEqual(await store.load(), []);
  } finally {
    await rm(rootDir, { recursive: true, force: true });
  }
});

test("persists and reloads input history", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-history-"));

  try {
    const store = new InputHistoryStore(rootDir);
    await store.append("first", 500);
    await store.append("/workflow deploy", 500);

    assert.deepEqual(await store.load(), ["first", "/workflow deploy"]);
    assert.match(await readFile(store.filePath, "utf-8"), /"version": 1/);
  } finally {
    await rm(rootDir, { recursive: true, force: true });
  }
});

test("preserves entries appended concurrently by separate stores", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-history-"));

  try {
    const firstStore = new InputHistoryStore(rootDir);
    const secondStore = new InputHistoryStore(rootDir);

    await Promise.all([
      firstStore.append("first", 500),
      secondStore.append("second", 500),
      firstStore.append("third", 500),
      secondStore.append("fourth", 500),
    ]);

    assert.deepEqual(
      (await firstStore.load()).toSorted(),
      ["first", "second", "third", "fourth"].toSorted(),
    );
  } finally {
    await rm(rootDir, { recursive: true, force: true });
  }
});

test("reclaims abandoned lock entries without disturbing concurrent writers", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-history-"));

  try {
    const store = new InputHistoryStore(rootDir);
    const lockDirectoryPath = `${store.filePath}.locks`;
    const abandonedProcess = spawn(process.execPath, ["-e", ""]);
    const abandonedPid = abandonedProcess.pid;
    assert.ok(abandonedPid);
    await once(abandonedProcess, "exit");

    await mkdir(lockDirectoryPath, { recursive: true });
    await writeFile(
      path.join(
        lockDirectoryPath,
        `${abandonedPid}-${randomUUID()}.choosing`,
      ),
      "",
    );
    await writeFile(
      path.join(
        lockDirectoryPath,
        `1-${abandonedPid}-${randomUUID()}.ticket`,
      ),
      "",
    );

    await Promise.all([
      new InputHistoryStore(rootDir).append("first", 500),
      new InputHistoryStore(rootDir).append("second", 500),
    ]);

    assert.deepEqual(
      (await store.load()).toSorted(),
      ["first", "second"].toSorted(),
    );
    assert.deepEqual(await readdir(lockDirectoryPath), []);
  } finally {
    await rm(rootDir, { recursive: true, force: true });
  }
});

test("retains only the configured number of entries", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-history-"));

  try {
    const store = new InputHistoryStore(rootDir);
    await store.append("first", 2);
    await store.append("second", 2);
    await store.append("third", 2);

    assert.deepEqual(await store.load(), ["second", "third"]);
  } finally {
    await rm(rootDir, { recursive: true, force: true });
  }
});

test("rejects malformed persisted history", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-history-"));

  try {
    const store = new InputHistoryStore(rootDir);
    await writeFile(
      store.filePath,
      JSON.stringify({ version: 1, entries: [42] }),
      "utf-8",
    );

    await assert.rejects(store.load(), InputHistoryStoreError);
  } finally {
    await rm(rootDir, { recursive: true, force: true });
  }
});
