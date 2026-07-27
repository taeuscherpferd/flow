import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AgentConversationStore } from "#src/agents/AgentConversationStore.js";
import { AgentManager } from "#src/agents/AgentManager.js";
import { ConfigService } from "#src/services/ConfigService.js";

test("switches project-scoped conversations and persists per-agent models", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-manager-"));
  const globalDir = path.join(root, "global");
  const projectDir = path.join(root, "project", ".work-agent");
  const config = new ConfigService({ globalDir, projectDir });
  let manager: AgentManager | undefined;
  try {
    await config.saveModelSetup({
      provider: "local",
      baseUrl: "http://localhost:11434",
      model: "default",
      contextWindow: 8192,
    });
    await writeFile(
      path.join(globalDir, "models.json"),
      JSON.stringify({
        defaultProvider: "local",
        defaultModel: "default",
        modelAliases: { finance: "local/finance-model" },
        providers: {
          local: {
            baseUrl: "http://localhost:11434",
            models: [
              { name: "default", contextWindow: 8192 },
              { name: "finance-model", contextWindow: 8192 },
            ],
          },
        },
      }),
    );
    const agentDir = path.join(globalDir, "agents", "finance");
    await mkdir(agentDir, { recursive: true });
    await writeFile(
      path.join(agentDir, "AGENT.yaml"),
      [
        "version: 1",
        "name: finance",
        "description: Manages finance",
        "model: finance",
      ].join("\n"),
    );
    await writeFile(path.join(agentDir, "SOUL.md"), "You are finance.");
    await writeFile(path.join(agentDir, "AGENTS.md"), "Be precise.");

    manager = await AgentManager.create(config);
    assert.equal(manager.getActiveName(), "main");
    assert.equal(manager.getActiveAgent().getCurrentModel().model, "default");
    const finance = await manager.switchAgent("finance");
    assert.equal(finance.getCurrentModel().model, "finance-model");
    assert.deepEqual(finance.setModel("default"), {
      ok: true,
      changed: true,
    });
    manager.persistActive();
    await manager.switchAgent("main");
    assert.equal(manager.getActiveAgent().getCurrentModel().model, "default");
    manager.close();

    manager = await AgentManager.create(config);
    await manager.switchAgent("finance");
    assert.equal(manager.getActiveAgent().getCurrentModel().model, "default");
  } finally {
    manager?.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("execution managers do not overwrite direct conversations", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-execution-manager-"));
  const globalDir = path.join(root, "global");
  const projectRoot = path.join(root, "project");
  const projectDir = path.join(projectRoot, ".work-agent");
  const config = new ConfigService({ globalDir, projectDir });
  let manager: AgentManager | undefined;
  try {
    await config.saveModelSetup({
      provider: "local",
      baseUrl: "http://localhost:11434",
      model: "default",
      contextWindow: 8192,
    });
    const session = {
      id: "main-session",
      projectDir: projectRoot,
      agentName: "main",
      mode: "direct" as const,
      provider: "local",
      model: "default",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    };
    let conversations = new AgentConversationStore(globalDir);
    conversations.save(session, [{ role: "user", content: "old history" }]);
    conversations.close();

    manager = await AgentManager.createExecution(config);
    conversations = new AgentConversationStore(globalDir);
    conversations.save(session, [{ role: "user", content: "new history" }]);
    conversations.close();
    manager.close();
    manager = undefined;

    conversations = new AgentConversationStore(globalDir);
    assert.deepEqual(conversations.get(projectRoot, "main")?.history, [
      { role: "user", content: "new history" },
    ]);
    conversations.close();
  } finally {
    manager?.close();
    await rm(root, { recursive: true, force: true });
  }
});
