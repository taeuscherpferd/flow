import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AgentPackageRegistry } from "#src/agents/AgentPackageRegistry.js";
import type { ModelsConfig } from "#src/services/ConfigService.js";

const models: ModelsConfig = {
  defaultProvider: "local",
  defaultModel: "default",
  modelAliases: { finance: "local/specialist" },
  providers: {
    local: {
      baseUrl: "http://localhost:11434",
      models: [
        { name: "default", contextWindow: 8192 },
        { name: "specialist", contextWindow: 16384 },
      ],
    },
  },
};

async function writeAgent(
  root: string,
  source: "global" | "project",
  soul: string,
): Promise<string> {
  const directory = path.join(root, source, "agents", "finance");
  await mkdir(path.join(directory, "skills", "reconcile-transactions"), {
    recursive: true,
  });
  await mkdir(path.join(directory, "context"), { recursive: true });
  await writeFile(
    path.join(directory, "AGENT.yaml"),
    [
      "version: 1",
      "name: finance",
      "description: Manages finance operations",
      "model: finance",
      "thinking: medium",
      "tools:",
      "  - read_file",
      "  - load_skill",
    ].join("\n"),
  );
  await writeFile(path.join(directory, "SOUL.md"), soul);
  await writeFile(path.join(directory, "AGENTS.md"), "Reconcile precisely.");
  await writeFile(path.join(directory, "CONTEXT.md"), "Use policy on demand.");
  await writeFile(
    path.join(directory, "context", "policy.md"),
    `${source} policy`,
  );
  await writeFile(
    path.join(
      directory,
      "skills",
      "reconcile-transactions",
      "SKILL.md",
    ),
    [
      "---",
      "name: reconcile-transactions",
      "description: Reconciles transactions",
      "---",
      "",
      `${source} instructions`,
    ].join("\n"),
  );
  return directory;
}

test("project agent packages replace global packages atomically", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-agents-"));
  try {
    await writeAgent(root, "global", "Global persona");
    const projectDirectory = await writeAgent(root, "project", "Project persona");
    const warnings: string[] = [];
    const registry = new AgentPackageRegistry(
      {
        globalDir: path.join(root, "global"),
        projectDir: path.join(root, "project"),
        models,
      },
      (warning) => warnings.push(warning),
    );

    await registry.load();

    const agent = registry.get("finance");
    assert.ok(agent);
    assert.equal(agent.directory, projectDirectory);
    assert.equal(agent.source, "project");
    assert.equal(agent.soul, "Project persona");
    assert.deepEqual(agent.contextFiles, ["context/policy.md"]);
    assert.equal(
      agent.skills[0]?.resourceId,
      "finance/reconcile-transactions",
    );
    assert.equal(agent.fingerprint.algorithm, "sha256");
    assert.equal(warnings.length, 0);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("fingerprints change when any package context file changes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-fingerprint-"));
  try {
    const directory = await writeAgent(root, "global", "Persona");
    const registry = new AgentPackageRegistry({
      globalDir: path.join(root, "global"),
      projectDir: path.join(root, "project"),
      models,
    });
    await registry.load();
    const before = registry.get("finance")!.fingerprint.value;

    await writeFile(path.join(directory, "context", "policy.md"), "changed");
    await registry.load();

    assert.notEqual(registry.get("finance")!.fingerprint.value, before);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("invalid manifests and missing required files are rejected", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-invalid-agent-"));
  try {
    const directory = path.join(root, "global", "agents", "Bad_Name");
    await mkdir(directory, { recursive: true });
    await writeFile(
      path.join(directory, "AGENT.yaml"),
      "version: 1\nname: Bad_Name\ndescription: invalid\n",
    );
    const warnings: string[] = [];
    const registry = new AgentPackageRegistry(
      {
        globalDir: path.join(root, "global"),
        projectDir: path.join(root, "project"),
        models,
      },
      (warning) => warnings.push(warning),
    );

    await registry.load();

    assert.equal(registry.list().length, 0);
    assert.match(warnings[0] ?? "", /lowercase kebab-case/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("an invalid project package does not fall back to the global package", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-agent-shadow-"));
  try {
    await writeAgent(root, "global", "Global persona");
    const projectDirectory = path.join(root, "project", "agents", "finance");
    await mkdir(projectDirectory, { recursive: true });
    await writeFile(
      path.join(projectDirectory, "AGENT.yaml"),
      "version: 2\nname: finance\ndescription: invalid override\n",
    );
    const registry = new AgentPackageRegistry(
      {
        globalDir: path.join(root, "global"),
        projectDir: path.join(root, "project"),
        models,
      },
      () => undefined,
    );

    await registry.load();

    assert.equal(registry.get("finance"), undefined);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects symbolic links anywhere in an agent package", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-agent-link-"));
  try {
    const directory = await writeAgent(root, "global", "Persona");
    const external = path.join(root, "external-policy.md");
    await writeFile(external, "external");
    await symlink(external, path.join(directory, "context", "linked-policy.md"));
    const warnings: string[] = [];
    const registry = new AgentPackageRegistry(
      {
        globalDir: path.join(root, "global"),
        projectDir: path.join(root, "project"),
        models,
      },
      (warning) => warnings.push(warning),
    );

    await registry.load();

    assert.equal(registry.get("finance"), undefined);
    assert.match(warnings[0] ?? "", /Symbolic links are not allowed/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
