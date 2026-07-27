import type { ThinkingMode } from "#src/providers/types.js";
import type { SkillRecord } from "#src/services/SkillsService.js";

export const AGENT_NAME_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

export type AgentExecutionMode =
  | "direct"
  | "delegated"
  | "workflow"
  | "scheduled";

export enum AgentToolName {
  ReadFile = "read_file",
  WriteFile = "write_file",
  RunCommand = "run_command",
  LoadSkill = "load_skill",
  RunWorkflow = "run_workflow",
  ListAgents = "list_agents",
  DelegateAgent = "delegate_agent",
  CreateSchedule = "create_schedule",
  ListSchedules = "list_schedules",
  PauseSchedule = "pause_schedule",
  ResumeSchedule = "resume_schedule",
  DeleteSchedule = "delete_schedule",
}

export interface AgentDefinition {
  version: 1;
  name: string;
  description: string;
  model?: string;
  thinking?: ThinkingMode;
  tools: AgentToolName[];
}

export interface AgentPackageFingerprint {
  algorithm: "sha256";
  value: string;
}

export interface AgentRecord {
  definition: AgentDefinition;
  directory: string;
  source: "global" | "project";
  soul: string;
  instructions: string;
  contextIndex?: string;
  contextFiles: string[];
  skills: SkillRecord[];
  fingerprint: AgentPackageFingerprint;
}

export interface AgentSessionRecord {
  id: string;
  projectDir: string;
  agentName: string;
  mode: AgentExecutionMode;
  provider: string;
  model: string;
  createdAt: string;
  updatedAt: string;
}

export interface AgentProfile {
  name: string;
  description: string;
  model?: string;
  thinking?: ThinkingMode;
  tools: AgentToolName[];
  soul: string;
  instructions: string;
  contextIndex?: string;
  contextFiles: string[];
  packageDirectory?: string;
  fingerprint?: AgentPackageFingerprint;
}
