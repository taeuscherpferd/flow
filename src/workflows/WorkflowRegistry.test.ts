import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { WorkflowRegistry } from "./WorkflowRegistry.js";

interface GeneratedWorkflowTsConfig {
  compilerOptions: {
    noUncheckedIndexedAccess?: boolean;
    paths: Record<string, string[]>;
  };
}

async function writeWorkflow(
  root: string,
  name: string,
  source: string,
  extension: "js" | "ts" = "js",
): Promise<string> {
  const directory = path.join(root, "workflows", name);
  await mkdir(directory, { recursive: true });
  const entry = path.join(directory, `WORKFLOW.${extension}`);
  await writeFile(entry, source, "utf-8");
  return entry;
}

const SIMPLE_WORKFLOW = `
  import { defineWorkflow } from "flowmation/workflow";
  export default defineWorkflow({
    name: "hello",
    description: "Returns a greeting",
    async run(_context, input) {
      return { greeting: input };
    },
  });
`;

test("loads JavaScript and TypeScript workflows through the virtual SDK", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-registry-"));
  const globalDir = path.join(root, "global");
  const projectDir = path.join(root, "project");

  try {
    await writeWorkflow(globalDir, "hello", SIMPLE_WORKFLOW);
    await writeWorkflow(
      globalDir,
      "typed",
      `
        import { defineWorkflow } from "flowmation/workflow";
        interface Input { value: string }
        export default defineWorkflow<Input, { result: string }>({
          name: "typed",
          description: "A typed workflow",
          input: {
            schema: {
              type: "object",
              properties: { value: { type: "string" } },
              required: ["value"],
              additionalProperties: false,
            },
          },
          async run(_context, input) {
            return { result: input.value };
          },
        });
      `,
      "ts",
    );
    await writeWorkflow(
      globalDir,
      "nested-types",
      `
        import { defineWorkflow } from "flowmation/workflow";
        export default defineWorkflow({
          name: "nested-types",
          description: "Uses every nested schema type",
          input: {
            schema: {
              type: "object",
              properties: {
                count: { type: "number" },
                enabled: { type: "boolean" },
                tags: { type: "array", items: { type: "string" } },
              },
              required: ["count", "enabled", "tags"],
              additionalProperties: false,
            },
          },
          async run(_context, input) {
            return input;
          },
        });
      `,
      "ts",
    );
    await writeFile(
      path.join(globalDir, "workflows", "loose.ts"),
      SIMPLE_WORKFLOW.replaceAll("hello", "loose"),
      "utf-8",
    );
    const registry = new WorkflowRegistry({ globalDir, projectDir });
    await registry.load();

    assert.deepEqual(
      registry.list().map((record) => record.definition.name).sort(),
      ["hello", "nested-types", "typed"],
    );
    assert.equal(registry.get("hello")?.source, "global");
    assert.deepEqual(
      registry.parseInput(registry.get("typed")!, '{"value":"ok"}'),
      { value: "ok" },
    );
    assert.throws(
      () => registry.parseInput(registry.get("typed")!, '{"extra":true}'),
      /input\.value is required/,
    );
    assert.deepEqual(
      registry.parseInput(
        registry.get("nested-types")!,
        '{"count":2,"enabled":true,"tags":["one","two"]}',
      ),
      { count: 2, enabled: true, tags: ["one", "two"] },
    );
    assert.throws(
      () => registry.parseInput(registry.get("typed")!, ""),
      /input\.value is required/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("project workflows override global workflows and ambiguous entries are skipped", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-registry-"));
  const globalDir = path.join(root, "global");
  const projectDir = path.join(root, "project");
  const warnings: string[] = [];

  try {
    await writeWorkflow(globalDir, "hello", SIMPLE_WORKFLOW);
    await writeWorkflow(
      projectDir,
      "hello",
      SIMPLE_WORKFLOW.replace("Returns a greeting", "Project greeting"),
    );
    await writeWorkflow(
      globalDir,
      "ambiguous",
      SIMPLE_WORKFLOW.replaceAll("hello", "ambiguous"),
    );
    await writeWorkflow(
      globalDir,
      "ambiguous",
      SIMPLE_WORKFLOW.replaceAll("hello", "ambiguous"),
      "ts",
    );

    const registry = new WorkflowRegistry(
      { globalDir, projectDir },
      (warning) => warnings.push(warning),
    );
    await registry.load();

    assert.equal(registry.get("hello")?.source, "project");
    assert.equal(registry.get("hello")?.definition.description, "Project greeting");
    assert.equal(registry.get("ambiguous"), undefined);
    assert.equal(warnings.some((warning) => warning.includes("both WORKFLOW")), true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("workflow fingerprints include every file in their directory", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "flowmation-registry-"));
  const globalDir = path.join(root, "global");
  const projectDir = path.join(root, "project");
  const workflowDir = path.join(globalDir, "workflows", "dependent");

  try {
    await mkdir(workflowDir, { recursive: true });
    await writeFile(
      path.join(workflowDir, "message.ts"),
      'export const message = "first";',
      "utf-8",
    );
    await writeWorkflow(
      globalDir,
      "dependent",
      `
        import { defineWorkflow } from "flowmation/workflow";
        import { message } from "./message.js";
        export default defineWorkflow({
          name: "dependent",
          description: "Uses a local helper",
          async run() {
            return message;
          },
        });
      `,
    );

    const registry = new WorkflowRegistry({ globalDir, projectDir });
    await registry.load();
    const firstFingerprint = registry.get("dependent")!.fingerprint;

    await writeFile(
      path.join(workflowDir, "message.ts"),
      'export const message = "second";',
      "utf-8",
    );
    await registry.load();

    assert.notEqual(
      registry.get("dependent")!.fingerprint,
      firstFingerprint,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("keeps project workflow SDK paths portable and refreshed", async () => {
  const root = await mkdtemp(
    path.join(process.cwd(), ".flowmation-registry-"),
  );
  const globalDir = path.join(root, "global");
  const projectDir = path.join(root, ".work-agent");
  const workflowsDir = path.join(projectDir, "workflows");

  try {
    await writeWorkflow(projectDir, "hello", SIMPLE_WORKFLOW);
    await writeFile(
      path.join(workflowsDir, "tsconfig.json"),
      JSON.stringify({
        compilerOptions: {
          strict: true,
          noUncheckedIndexedAccess: true,
          paths: {
            "flowmation/workflow": ["D:/stale/flowmation/sdk.ts"],
          },
        },
      }),
      "utf-8",
    );

    const registry = new WorkflowRegistry({ globalDir, projectDir });
    await registry.load();
    const generatedTsConfig = JSON.parse(
      await readFile(path.join(workflowsDir, "tsconfig.json"), "utf-8"),
    ) as GeneratedWorkflowTsConfig;
    const sdkReference =
      generatedTsConfig.compilerOptions.paths["flowmation/workflow"]?.[0];

    assert.ok(sdkReference);
    assert.equal(path.isAbsolute(sdkReference), false);
    assert.equal(
      generatedTsConfig.compilerOptions.noUncheckedIndexedAccess,
      true,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
