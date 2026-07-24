import { spawn, type ChildProcess } from "node:child_process";

function terminateWindowsProcessTree(processId: number): Promise<boolean> {
  return new Promise((resolve) => {
    const taskkill = spawn(
      "taskkill.exe",
      ["/pid", String(processId), "/t", "/f"],
      {
        windowsHide: true,
        stdio: "ignore",
      },
    );
    taskkill.once("error", () => resolve(false));
    taskkill.once("close", (code) => resolve(code === 0));
  });
}

export async function terminateProcessTree(child: ChildProcess): Promise<void> {
  const processId = child.pid;
  if (processId === undefined) {
    child.kill("SIGKILL");
    return;
  }

  if (process.platform === "win32") {
    if (!(await terminateWindowsProcessTree(processId))) {
      child.kill("SIGKILL");
    }
    return;
  }

  try {
    process.kill(-processId, "SIGKILL");
  } catch {
    child.kill("SIGKILL");
  }
}
