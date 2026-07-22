import { exec } from "node:child_process";
import { promisify } from "node:util";
import type { Tool } from "./types.js";

const execAsync = promisify(exec);

const TIMEOUT_MS = 30_000;
const MAX_OUTPUT_CHARS = 20_000;

interface ExecError extends Error {
  stdout?: string;
  stderr?: string;
  code?: number;
  killed?: boolean;
  signal?: string;
}

function truncate(text: string): { text: string; truncated: boolean } {
  if (text.length <= MAX_OUTPUT_CHARS) return { text, truncated: false };
  return { text: text.slice(0, MAX_OUTPUT_CHARS), truncated: true };
}

function formatOutput(stdout: string, stderr: string, exitCode: number | null): string {
  const out = truncate(stdout);
  const err = truncate(stderr);
  const parts = [`exit code: ${exitCode ?? "unknown"}`];
  parts.push(`stdout:\n${out.text}${out.truncated ? "\n[stdout truncated]" : ""}`);
  if (err.text.length > 0) {
    parts.push(`stderr:\n${err.text}${err.truncated ? "\n[stderr truncated]" : ""}`);
  }
  return parts.join("\n\n");
}

export const runCommandTool: Tool = {
  name: "run_command",
  description:
    "Execute a shell command and return its stdout, stderr, and exit code. Times out after 30 seconds; output is capped at 20KB.",
  parameters: {
    type: "object",
    properties: {
      command: { type: "string", description: "The shell command to execute." },
    },
    required: ["command"],
  },
  async execute(args, ctx) {
    const command = args["command"];
    if (typeof command !== "string" || command.trim().length === 0) {
      return { ok: false, content: "Error: 'command' must be a non-empty string." };
    }

    try {
      const { stdout, stderr } = await execAsync(command, {
        cwd: ctx.cwd,
        timeout: TIMEOUT_MS,
        maxBuffer: 10 * 1024 * 1024,
      });
      return { ok: true, content: formatOutput(stdout, stderr, 0) };
    } catch (err) {
      const e = err as ExecError;
      const body = formatOutput(e.stdout ?? "", e.stderr ?? "", e.code ?? null);
      if (e.killed && e.signal === "SIGTERM") {
        return { ok: false, content: `Command timed out after ${TIMEOUT_MS / 1000}s.\n\n${body}` };
      }
      return { ok: false, content: body };
    }
  },
};
