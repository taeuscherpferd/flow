import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AgentConversationStore } from "#src/agents/AgentConversationStore.js";

test("persists separate project-scoped agent conversations without system messages", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-conversations-"));
  const store = new AgentConversationStore(root);
  try {
    const now = new Date().toISOString();
    store.save(
      {
        id: "finance-session",
        projectDir: "/project-a",
        agentName: "finance",
        mode: "direct",
        provider: "local",
        model: "finance-model",
        createdAt: now,
        updatedAt: now,
      },
      [
        { role: "system", content: "stale system prompt" },
        { role: "user", content: "project A question" },
        { role: "assistant", content: "project A answer" },
      ],
    );
    store.save(
      {
        id: "finance-project-b",
        projectDir: "/project-b",
        agentName: "finance",
        mode: "direct",
        provider: "local",
        model: "finance-model",
        createdAt: now,
        updatedAt: now,
      },
      [{ role: "user", content: "project B question" }],
    );

    assert.deepEqual(store.get("/project-a", "finance")?.history, [
      { role: "user", content: "project A question" },
      { role: "assistant", content: "project A answer" },
    ]);
    assert.deepEqual(store.get("/project-b", "finance")?.history, [
      { role: "user", content: "project B question" },
    ]);
    assert.equal(store.get("/project-a", "main"), undefined);
  } finally {
    store.close();
    await rm(root, { recursive: true, force: true });
  }
});
