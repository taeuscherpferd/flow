import { createHash } from "node:crypto";
import { access, readdir, readFile, writeFile } from "node:fs/promises";
import { registerHooks } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { tsImport } from "tsx/esm/api";
import { validateSchema } from "#src/workflows/schema.js";
import type {
  AgentInvocationPolicy,
  JsonValue,
  WorkflowDefinition,
  WorkflowPresentation,
  WorkflowRecord,
} from "#src/workflows/types.js";

const WORKFLOW_NAME_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const ENTRY_NAMES = ["WORKFLOW.ts", "WORKFLOW.js"] as const;
const SDK_SPECIFIER = "flowmation/workflow";

let sdkHookRegistered = false;

interface WorkflowModule {
  default?: WorkflowDefinition<JsonValue, JsonValue>;
}

function isJsonObject(
  value: JsonValue | undefined,
): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function registerSdkHook(): void {
  if (sdkHookRegistered) return;

  const extension = import.meta.url.endsWith(".ts") ? "ts" : "js";
  const sdkUrl = new URL(`./sdk.${extension}`, import.meta.url).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === SDK_SPECIFIER) {
        return { url: sdkUrl, shortCircuit: true };
      }
      return nextResolve(specifier, context);
    },
  });
  sdkHookRegistered = true;
}

async function collectFiles(directory: string, relative = ""): Promise<string[]> {
  const current = path.join(directory, relative);
  const entries = await readdir(current, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const childRelative = path.join(relative, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectFiles(directory, childRelative)));
    } else if (entry.isFile()) {
      files.push(childRelative);
    }
  }

  return files;
}

async function fingerprintDirectory(directory: string): Promise<string> {
  const hash = createHash("sha256");
  const files = (await collectFiles(directory)).sort((left, right) =>
    left.localeCompare(right),
  );

  for (const file of files) {
    hash.update(file.replaceAll(path.sep, "/"));
    hash.update("\0");
    hash.update(await readFile(path.join(directory, file)));
    hash.update("\0");
  }

  return hash.digest("hex");
}

async function ensureEsmScope(configDir: string): Promise<void> {
  const workflowsDir = path.join(configDir, "workflows");
  try {
    await access(workflowsDir);
  } catch {
    return;
  }

  const packagePath = path.join(configDir, "package.json");
  try {
    await access(packagePath);
  } catch {
    await writeFile(
      packagePath,
      JSON.stringify({ private: true, type: "module" }, null, 2),
      "utf-8",
    );
  }

  const tsconfigPath = path.join(workflowsDir, "tsconfig.json");
  const extension = import.meta.url.endsWith(".ts") ? "ts" : "d.ts";
  const sdkPath = fileURLToPath(new URL(`./sdk.${extension}`, import.meta.url));
  const relativeSdkPath = path.relative(workflowsDir, sdkPath);
  const sdkReference = path.isAbsolute(relativeSdkPath)
    ? sdkPath.replaceAll(path.sep, "/")
    : relativeSdkPath.replaceAll(path.sep, "/");
  let tsconfigExists = true;
  try {
    await access(tsconfigPath);
  } catch {
    tsconfigExists = false;
  }
  if (tsconfigExists) {
    let config: JsonValue;
    try {
      config = JSON.parse(await readFile(tsconfigPath, "utf-8")) as JsonValue;
    } catch {
      return;
    }
    if (!isJsonObject(config)) return;
    const compilerOptions = config["compilerOptions"];
    if (!isJsonObject(compilerOptions)) return;
    const paths = compilerOptions["paths"];
    if (!isJsonObject(paths) || paths[SDK_SPECIFIER] === undefined) return;
    paths[SDK_SPECIFIER] = [sdkReference];
    await writeFile(tsconfigPath, JSON.stringify(config, null, 2), "utf-8");
    return;
  }

  await writeFile(
    tsconfigPath,
    JSON.stringify(
      {
        compilerOptions: {
          target: "ESNext",
          module: "NodeNext",
          moduleResolution: "NodeNext",
          strict: true,
          types: ["node"],
          paths: {
            [SDK_SPECIFIER]: [sdkReference],
          },
        },
        include: ["./**/*.ts"],
      },
      null,
      2,
    ),
    "utf-8",
  );
}

function validateDefinition(
  definition: WorkflowDefinition<JsonValue, JsonValue> | undefined,
  expectedName: string,
): string | undefined {
  if (!definition || typeof definition !== "object") {
    return "the module must default-export a workflow definition";
  }
  if (typeof definition.name !== "string" || definition.name.length === 0) {
    return 'the definition is missing a non-empty "name"';
  }
  if (definition.name !== expectedName) {
    return `the exported name "${definition.name}" does not match directory "${expectedName}"`;
  }
  if (!WORKFLOW_NAME_PATTERN.test(definition.name)) {
    return "workflow names must use lowercase kebab-case";
  }
  if (
    typeof definition.description !== "string" ||
    definition.description.trim().length === 0
  ) {
    return 'the definition is missing a non-empty "description"';
  }
  if (typeof definition.run !== "function") {
    return 'the definition is missing a "run" function';
  }

  const invocation = definition.agentInvocation;
  const validInvocations: AgentInvocationPolicy[] = [
    "disabled",
    "confirm",
    "automatic",
  ];
  if (invocation !== undefined && !validInvocations.includes(invocation)) {
    return `"agentInvocation" must be disabled, confirm, or automatic`;
  }

  const presentation = definition.presentation;
  const validPresentations: WorkflowPresentation[] = ["direct", "agent"];
  if (
    presentation !== undefined &&
    !validPresentations.includes(presentation)
  ) {
    return `"presentation" must be direct or agent`;
  }

  if (
    definition.input !== undefined &&
    (typeof definition.input !== "object" ||
      !definition.input.schema ||
      (definition.input.schema.type !== "string" &&
        definition.input.schema.type !== "object"))
  ) {
    return `"input.schema" must be a string or object schema`;
  }

  return undefined;
}

export interface WorkflowRegistryDirectories {
  globalDir: string;
  projectDir: string;
}

export class WorkflowRegistry {
  private readonly workflows = new Map<string, WorkflowRecord>();

  constructor(
    private readonly directories: WorkflowRegistryDirectories,
    private readonly warn: (message: string) => void = (message) =>
      console.warn(message),
  ) {
    registerSdkHook();
  }

  async load(): Promise<void> {
    this.workflows.clear();
    await ensureEsmScope(this.directories.globalDir);
    await ensureEsmScope(this.directories.projectDir);
    await this.scan(
      path.join(this.directories.globalDir, "workflows"),
      "global",
    );
    await this.scan(
      path.join(this.directories.projectDir, "workflows"),
      "project",
    );
  }

  list(): WorkflowRecord[] {
    return Array.from(this.workflows.values());
  }

  get(name: string): WorkflowRecord | undefined {
    return this.workflows.get(name);
  }

  parseInput(record: WorkflowRecord, raw: string): JsonValue {
    const schema = record.definition.input?.schema;
    if (!schema || schema.type === "string") {
      const result: JsonValue = raw;
      if (schema) {
        const validation = validateSchema(schema, result);
        if (!validation.valid) throw new Error(validation.errors.join("\n"));
      }
      return result;
    }

    let input: JsonValue;
    try {
      input = raw.length === 0 ? {} : (JSON.parse(raw) as JsonValue);
    } catch (error) {
      throw new Error(
        `Workflow "${record.definition.name}" expects JSON object input: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }

    const validation = validateSchema(schema, input);
    if (!validation.valid) throw new Error(validation.errors.join("\n"));
    return input;
  }

  validateInput(record: WorkflowRecord, input: JsonValue): void {
    const schema = record.definition.input?.schema;
    if (!schema) {
      if (typeof input !== "string") {
        throw new TypeError(
          `Workflow "${record.definition.name}" expects string input.`,
        );
      }
      return;
    }

    const validation = validateSchema(schema, input);
    if (!validation.valid) throw new Error(validation.errors.join("\n"));
  }

  private async scan(
    workflowsDir: string,
    source: "global" | "project",
  ): Promise<void> {
    let entries;
    try {
      entries = await readdir(workflowsDir, { withFileTypes: true });
    } catch {
      return;
    }

    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      await this.loadDirectory(workflowsDir, entry.name, source);
    }
  }

  private async loadDirectory(
    workflowsDir: string,
    name: string,
    source: "global" | "project",
  ): Promise<void> {
    if (!WORKFLOW_NAME_PATTERN.test(name)) {
      this.warn(
        `Skipping workflow directory "${name}" — names must use lowercase kebab-case.`,
      );
      return;
    }

    const directory = path.join(workflowsDir, name);
    const contents = await readdir(directory);
    const entryNames = ENTRY_NAMES.filter((entry) => contents.includes(entry));
    if (entryNames.length === 0) return;
    if (entryNames.length > 1) {
      this.warn(
        `Skipping workflow "${name}" — both WORKFLOW.ts and WORKFLOW.js exist.`,
      );
      return;
    }

    const entryPath = path.join(directory, entryNames[0]!);
    await this.loadEntry(
      entryPath,
      directory,
      name,
      source,
      () => fingerprintDirectory(directory),
    );
  }

  private async loadEntry(
    entryPath: string,
    directory: string,
    name: string,
    source: "global" | "project",
    fingerprint: () => Promise<string>,
  ): Promise<void> {
    try {
      const module = (await tsImport(
        pathToFileURL(entryPath).href,
        import.meta.url,
      )) as WorkflowModule;
      const validationError = validateDefinition(module.default, name);
      if (validationError) {
        this.warn(`Skipping workflow "${name}" — ${validationError}.`);
        return;
      }

      this.workflows.set(name, {
        definition: module.default!,
        directory,
        entryPath,
        fingerprint: await fingerprint(),
        source,
      });
    } catch (error) {
      this.warn(
        `Skipping workflow "${name}" — failed to load ${entryPath}: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
  }
}
