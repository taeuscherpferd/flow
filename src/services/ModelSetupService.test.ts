import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ConfigService } from "./ConfigService.js";
import {
  ModelSetupService,
  type SetupPrompt,
} from "./ModelSetupService.js";
import { EOF } from "../ui/lineEditor.js";

interface TestSetup {
  rootDir: string;
  configService: ConfigService;
}

async function createTestSetup(): Promise<TestSetup> {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "flowmation-setup-"));
  return {
    rootDir,
    configService: new ConfigService({
      globalDir: path.join(rootDir, "global"),
      projectDir: path.join(rootDir, "project"),
    }),
  };
}

function scriptedPrompt(
  answers: Array<string | typeof EOF>,
): SetupPrompt {
  let answerIndex = 0;
  return async () => answers[answerIndex++] ?? EOF;
}

test("creates the first provider and model with validated answers", async () => {
  const testSetup = await createTestSetup();
  const output: string[] = [];
  const prompt = scriptedPrompt([
    "",
    "not-a-url",
    "http://localhost:11434/",
    "",
    "llama3.2",
    "0",
    "8192",
  ]);
  const service = new ModelSetupService(
    testSetup.configService,
    prompt,
    (message) => output.push(message),
  );

  try {
    const result = await service.run();
    const config = await testSetup.configService.load();

    assert.deepEqual(result, {
      status: "completed",
      configPath: path.join(testSetup.rootDir, "global", "models.json"),
      provider: "ollama",
      model: "llama3.2",
    });
    assert.equal(config.models.defaultProvider, "ollama");
    assert.equal(config.models.defaultModel, "llama3.2");
    assert.deepEqual(config.models.providers["ollama"], {
      baseUrl: "http://localhost:11434",
      models: [{ name: "llama3.2", contextWindow: 8192 }],
    });
    assert.ok(output.includes("Enter a valid http:// or https:// URL."));
    assert.ok(output.includes("Model name is required."));
    assert.ok(
      output.includes("Context window must be a positive whole number."),
    );
  } finally {
    await rm(testSetup.rootDir, { recursive: true, force: true });
  }
});

test("cancels setup when input closes", async () => {
  const testSetup = await createTestSetup();
  const service = new ModelSetupService(
    testSetup.configService,
    scriptedPrompt([EOF]),
    () => {},
  );

  try {
    assert.deepEqual(await service.run(), { status: "cancelled" });
  } finally {
    await rm(testSetup.rootDir, { recursive: true, force: true });
  }
});
