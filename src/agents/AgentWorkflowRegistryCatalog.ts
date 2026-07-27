import { readdir } from "node:fs/promises";
import path from "node:path";
import {
  fingerprintAgentPackage,
  type AgentPackageRegistry,
} from "#src/agents/AgentPackageRegistry.js";
import type { ResolvedConfig } from "#src/services/ConfigService.js";
import { WorkflowRegistry } from "#src/workflows/WorkflowRegistry.js";

export interface AgentExecutionScope {
  agentName: string;
  workflowName: string;
  packageFingerprint: string;
}

interface WorkflowRoot {
  directory: string;
  source: "global" | "project";
}

async function hasSingleWorkflowEntry(
  configDir: string,
  workflowName: string,
): Promise<boolean> {
  try {
    const entries = await readdir(
      path.join(configDir, "workflows", workflowName),
    );
    return ["WORKFLOW.ts", "WORKFLOW.js"].filter((name) =>
      entries.includes(name),
    ).length === 1;
  } catch {
    return false;
  }
}

async function scopedMainRoots(
  config: ResolvedConfig,
  executionScope?: AgentExecutionScope,
): Promise<WorkflowRoot[] | undefined> {
  if (executionScope === undefined) return undefined;
  const projectHasEntry = await hasSingleWorkflowEntry(
    config.projectDir,
    executionScope.workflowName,
  );
  return projectHasEntry
    ? [
        {
          directory: path.join(config.projectDir, "workflows"),
          source: "project",
        },
      ]
    : [
        {
          directory: path.join(config.globalDir, "workflows"),
          source: "global",
        },
      ];
}

async function mainRegistry(
  config: ResolvedConfig,
  executionScope?: AgentExecutionScope,
): Promise<WorkflowRegistry> {
  const scopedRoots = await scopedMainRoots(config, executionScope);
  return new WorkflowRegistry({
    globalDir: config.globalDir,
    projectDir: config.projectDir,
    agentName: "main",
    ...(scopedRoots === undefined ? {} : { roots: scopedRoots }),
    ...(executionScope === undefined
      ? {}
      : {
          names: [executionScope.workflowName],
          authorizeImport: async ({
            name,
            fingerprint,
          }: {
            name: string;
            directory: string;
            fingerprint: string;
          }) =>
            name === executionScope.workflowName &&
            fingerprint === executionScope.packageFingerprint,
        }),
  });
}

export async function createAgentWorkflowRegistries(
  config: ResolvedConfig,
  packageRegistry: AgentPackageRegistry,
  executionScope?: AgentExecutionScope,
): Promise<Map<string, WorkflowRegistry>> {
  const registries = new Map<string, WorkflowRegistry>();
  if (!executionScope || executionScope.agentName === "main") {
    const registry = await mainRegistry(config, executionScope);
    await registry.load();
    registries.set("main", registry);
  }

  const records =
    executionScope === undefined
      ? packageRegistry.list()
      : packageRegistry
          .list()
          .filter(
            (record) => record.definition.name === executionScope.agentName,
          );
  for (const record of records) {
    const registry = new WorkflowRegistry({
      globalDir: record.directory,
      projectDir: record.directory,
      roots: [
        {
          directory: path.join(record.directory, "workflows"),
          source: record.source,
        },
      ],
      agentName: record.definition.name,
      ...(executionScope === undefined
        ? {}
        : {
            names: [executionScope.workflowName],
            authorizeImport: async () =>
              (
                await fingerprintAgentPackage(record.directory)
              ).value === executionScope.packageFingerprint,
          }),
    });
    await registry.load();
    registries.set(record.definition.name, registry);
  }

  return registries;
}
