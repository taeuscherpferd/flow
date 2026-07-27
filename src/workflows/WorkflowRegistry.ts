import { readdir } from "node:fs/promises";
import { registerHooks } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { tsImport } from "tsx/esm/api";
import { fingerprintDirectory } from "#src/services/DirectoryFingerprint.js";
import { validateSchema } from "#src/workflows/schema.js";
import { prepareWorkflowEsmScope as prepareEsmScope } from "#src/workflows/WorkflowEsmScope.js";
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

export async function prepareWorkflowEsmScope(
  configDir: string,
): Promise<void> {
  const extension = import.meta.url.endsWith(".ts") ? "ts" : "d.ts";
  const sdkPath = fileURLToPath(new URL(`./sdk.${extension}`, import.meta.url));
  await prepareEsmScope(configDir, sdkPath);
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
  roots?: Array<{
    directory: string;
    source: "global" | "project";
  }>;
  agentName?: string;
  names?: readonly string[];
  authorizeImport?(record: {
    name: string;
    directory: string;
    fingerprint: string;
  }): Promise<boolean>;
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
    await prepareWorkflowEsmScope(this.directories.globalDir);
    if (this.directories.projectDir !== this.directories.globalDir) {
      await prepareWorkflowEsmScope(this.directories.projectDir);
    }
    const roots = this.directories.roots ?? [
      {
        directory: path.join(this.directories.globalDir, "workflows"),
        source: "global" as const,
      },
      {
        directory: path.join(this.directories.projectDir, "workflows"),
        source: "project" as const,
      },
    ];
    for (const root of roots) {
      await this.scan(root.directory, root.source);
    }
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
      if (
        this.directories.names &&
        !this.directories.names.includes(entry.name)
      ) {
        continue;
      }
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
      const sourceFingerprint = await fingerprint();
      if (
        this.directories.authorizeImport &&
        !(await this.directories.authorizeImport({
          name,
          directory,
          fingerprint: sourceFingerprint,
        }))
      ) {
        this.warn(
          `Skipping workflow "${name}" — its source is not authorized for this execution.`,
        );
        return;
      }
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
        fingerprint: sourceFingerprint,
        source,
        ...(this.directories.agentName === undefined
          ? {}
          : {
              agentName: this.directories.agentName,
              resourceId: `${this.directories.agentName}/${name}`,
            }),
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
