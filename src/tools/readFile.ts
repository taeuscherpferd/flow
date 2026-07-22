import { readFile } from "node:fs/promises";
import path from "node:path";
import type { Tool } from "./types.js";

export const readFileTool: Tool = {
  name: "read_file",
  description: "Read the contents of a text file at the given path.",
  parameters: {
    type: "object",
    properties: {
      path: {
        type: "string",
        description: "File path, absolute or relative to the current working directory.",
      },
    },
    required: ["path"],
  },
  async execute(args, ctx) {
    const filePath = args["path"];
    if (typeof filePath !== "string" || filePath.trim().length === 0) {
      return { ok: false, content: "Error: 'path' must be a non-empty string." };
    }

    const resolved = path.resolve(ctx.cwd, filePath);
    try {
      const content = await readFile(resolved, "utf-8");
      return { ok: true, content };
    } catch (err) {
      return { ok: false, content: `Error reading "${filePath}": ${String(err)}` };
    }
  },
};
