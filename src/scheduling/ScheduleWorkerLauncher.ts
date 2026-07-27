import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

export function ensureScheduleWorker(globalDir: string): void {
  const entryUrl = new URL("./ScheduleWorkerProcess.js", import.meta.url);
  const sourceMode = import.meta.url.endsWith(".ts");
  const entryPath = sourceMode
    ? fileURLToPath(new URL("./ScheduleWorkerProcess.ts", import.meta.url))
    : fileURLToPath(entryUrl);
  const args = sourceMode
    ? ["--import", "tsx", entryPath, globalDir]
    : [entryPath, globalDir];
  const child = spawn(process.execPath, args, {
    detached: true,
    stdio: "ignore",
  });
  child.unref();
}
