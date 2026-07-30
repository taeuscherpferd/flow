import { lstat, readFile, writeFile } from "node:fs/promises";
import { registerHooks } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { isJsonObject } from "./json.js";
import type {
  AgentInvocationPolicy,
  JsonObject,
  JsonValue,
  WorkflowDefinition,
  WorkflowPresentation,
  WorkflowRootSchema,
} from "./types.js";

const SDK_SPECIFIER = "flowmation/workflow";
const WORKFLOW_NAME_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const ENTRY_PATTERN = /^WORKFLOW\.(?:js|ts)$/;

interface WorkflowModule {
  default?:
    | WorkflowDefinition<JsonValue, JsonValue>
    | { default?: WorkflowDefinition<JsonValue, JsonValue> };
}

export interface LoadedWorkflow {
  definition: WorkflowDefinition<JsonValue, JsonValue>;
  entryPath: string;
}

const sdkExtension = import.meta.url.endsWith(".ts") ? "ts" : "js";
const sdkUrl = new URL(`./sdk.${sdkExtension}`, import.meta.url).href;

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === SDK_SPECIFIER) {
      return { url: sdkUrl, shortCircuit: true };
    }
    return nextResolve(specifier, context);
  },
});

export async function loadWorkflow(
  requestedEntryPath: string,
): Promise<LoadedWorkflow> {
  const entryPath = path.resolve(requestedEntryPath);
  if (!ENTRY_PATTERN.test(path.basename(entryPath))) {
    throw new Error(
      `Workflow entry must be named WORKFLOW.js or WORKFLOW.ts: ${entryPath}`,
    );
  }
  const directory = path.dirname(entryPath);
  const [entryStats, directoryStats] = await Promise.all([
    lstat(entryPath),
    lstat(directory),
  ]);
  if (
    entryStats.isSymbolicLink() ||
    directoryStats.isSymbolicLink() ||
    !entryStats.isFile() ||
    !directoryStats.isDirectory()
  ) {
    throw new Error(`Workflow paths must be regular, non-symbolic paths.`);
  }

  await ensureWorkflowSdkPath(entryPath);
  const entryUrl = pathToFileURL(entryPath).href;
  const imported = entryPath.endsWith(".ts")
    ? ((await import("tsx/esm/api")).tsImport(
        entryUrl,
        import.meta.url,
      ) as Promise<WorkflowModule>)
    : (import(entryUrl) as Promise<WorkflowModule>);
  const workflowModule = await imported;
  const exported = workflowModule.default;
  let definition: WorkflowDefinition<JsonValue, JsonValue> | undefined;
  if (
    exported &&
    typeof exported === "object" &&
    "default" in exported
  ) {
    definition = exported.default;
  } else {
    definition = exported as
      | WorkflowDefinition<JsonValue, JsonValue>
      | undefined;
  }
  validateDefinition(definition, path.basename(directory));
  return { definition, entryPath };
}

async function ensureWorkflowSdkPath(entryPath: string): Promise<void> {
  const workflowsDirectory = path.dirname(path.dirname(entryPath));
  const configPath = path.join(workflowsDirectory, "tsconfig.json");
  let config: JsonObject = {};
  try {
    const parsed = JSON.parse(await readFile(configPath, "utf-8")) as JsonValue;
    if (!isJsonObject(parsed)) {
      throw new TypeError(`${configPath} must contain a JSON object.`);
    }
    config = parsed;
  } catch (error) {
    if (!(error instanceof Error) || Reflect.get(error, "code") !== "ENOENT") {
      throw error;
    }
  }
  const configuredCompilerOptions = config["compilerOptions"] ?? null;
  const compilerOptions: JsonObject = isJsonObject(
    configuredCompilerOptions,
  )
    ? configuredCompilerOptions
    : {};
  const configuredPaths = compilerOptions["paths"] ?? null;
  const paths: JsonObject = isJsonObject(configuredPaths)
    ? configuredPaths
    : {};
  let sdkReference = path.relative(
    workflowsDirectory,
    fileURLToPath(sdkUrl),
  );
  if (!sdkReference.startsWith(".")) sdkReference = `./${sdkReference}`;
  sdkReference = sdkReference.split(path.sep).join("/");
  const refreshed: JsonObject = {
    ...config,
    compilerOptions: {
      ...compilerOptions,
      paths: {
        ...paths,
        [SDK_SPECIFIER]: [sdkReference],
      },
    },
  };
  await writeFile(configPath, `${JSON.stringify(refreshed, null, 2)}\n`, {
    encoding: "utf-8",
    mode: 0o600,
  });
}

function validateDefinition(
  definition: WorkflowDefinition<JsonValue, JsonValue> | undefined,
  expectedName: string,
): asserts definition is WorkflowDefinition<JsonValue, JsonValue> {
  if (!definition || typeof definition !== "object") {
    throw new TypeError(
      "The module must default-export a workflow definition.",
    );
  }
  if (
    typeof definition.name !== "string" ||
    !WORKFLOW_NAME_PATTERN.test(definition.name)
  ) {
    throw new TypeError("Workflow names must use lowercase kebab-case.");
  }
  if (definition.name !== expectedName) {
    throw new TypeError(
      `The exported name "${definition.name}" does not match directory "${expectedName}".`,
    );
  }
  if (
    typeof definition.description !== "string" ||
    definition.description.trim().length === 0
  ) {
    throw new TypeError(
      'The definition is missing a non-empty "description".',
    );
  }
  if (typeof definition.run !== "function") {
    throw new TypeError('The definition is missing a "run" function.');
  }
  validateInvocation(definition.agentInvocation);
  validatePresentation(definition.presentation);
  validateInputSchema(definition.input?.schema);
}

function validateInvocation(
  invocation: AgentInvocationPolicy | undefined,
): void {
  if (
    invocation !== undefined &&
    invocation !== "disabled" &&
    invocation !== "confirm" &&
    invocation !== "automatic"
  ) {
    throw new TypeError(
      '"agentInvocation" must be disabled, confirm, or automatic.',
    );
  }
}

function validatePresentation(
  presentation: WorkflowPresentation | undefined,
): void {
  if (
    presentation !== undefined &&
    presentation !== "direct" &&
    presentation !== "agent"
  ) {
    throw new TypeError('"presentation" must be direct or agent.');
  }
}

function validateInputSchema(schema: WorkflowRootSchema | undefined): void {
  if (
    schema !== undefined &&
    schema.type !== "string" &&
    schema.type !== "object"
  ) {
    throw new TypeError(
      '"input.schema" must be a string or object schema.',
    );
  }
}
