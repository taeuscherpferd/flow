import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  ConfigService,
  type ModelsConfig,
} from "./ConfigService.js";

interface TestConfig {
  rootDir: string;
  service: ConfigService;
}

async function createTestConfig(): Promise<TestConfig> {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-config-"));
  return {
    rootDir,
    service: new ConfigService({
      globalDir: path.join(rootDir, "global"),
      projectDir: path.join(rootDir, "project"),
    }),
  };
}

test("loads the first-run scaffold without requiring a model", async () => {
  const testConfig = await createTestConfig();

  try {
    const config = await testConfig.service.load();

    assert.equal(
      testConfig.service.hasConfiguredDefaultModel(config.models),
      false,
    );
    assert.equal(config.models.defaultProvider, "ollama");
    assert.deepEqual(config.models.providers["ollama"]?.models, []);
  } finally {
    await rm(testConfig.rootDir, { recursive: true, force: true });
  }
});

test("saves a model setup as the active global model", async () => {
  const testConfig = await createTestConfig();

  try {
    const configPath = await testConfig.service.saveModelSetup({
      provider: "local",
      baseUrl: "http://localhost:11434",
      model: "qwen3:8b",
      contextWindow: 16384,
    });
    const config = await testConfig.service.load();
    const persisted = JSON.parse(
      await readFile(configPath, "utf-8"),
    ) as ModelsConfig;

    assert.equal(
      testConfig.service.hasConfiguredDefaultModel(config.models),
      true,
    );
    assert.equal(persisted.defaultProvider, "local");
    assert.equal(persisted.defaultModel, "qwen3:8b");
    assert.deepEqual(persisted.providers["local"], {
      baseUrl: "http://localhost:11434",
      models: [{ name: "qwen3:8b", contextWindow: 16384 }],
    });
  } finally {
    await rm(testConfig.rootDir, { recursive: true, force: true });
  }
});
