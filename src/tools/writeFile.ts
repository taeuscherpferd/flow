import { mkdir, writeFile as fsWriteFile } from "node:fs/promises";
import path from "node:path";
import type { Tool } from "./types.js";

export const writeFileTool: Tool = {
  name: "write_file",
  description:
    "Write content to a file at the given path, creating parent directories as needed. Overwrites existing files.",
  parameters: {
    type: "object",
    properties: {
      path: {
        type: "string",
        description: "File path, absolute or relative to the current working directory.",
      },
      content: {
        type: "string",
        description: "The full content to write to the file.",
      },
    },
    required: ["path", "content"],
  },
  async execute(args, ctx) {
    const filePath = args["path"];
    const content = args["content"];
    if (typeof filePath !== "string" || filePath.trim().length === 0) {
      return { ok: false, content: "Error: 'path' must be a non-empty string." };
    }
    if (typeof content !== "string") {
      return { ok: false, content: "Error: 'content' must be a string." };
    }

    const resolved = path.resolve(ctx.cwd, filePath);
    try {
      await mkdir(path.dirname(resolved), { recursive: true });
      await fsWriteFile(resolved, content, "utf-8");
      return { ok: true, content: `Wrote ${content.length} characters to "${filePath}".` };
    } catch (err) {
      return { ok: false, content: `Error writing "${filePath}": ${String(err)}` };
    }
  },
};
