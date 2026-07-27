import { lstat, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import type { JsonValue } from "#src/workflows/types.js";

const SDK_SPECIFIER = "flowmation/workflow";

function isJsonObject(
  value: JsonValue | undefined,
): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function existingRegularPath(target: string): Promise<boolean> {
  try {
    const stats = await lstat(target);
    if (stats.isSymbolicLink()) {
      throw new Error(
        `Symbolic workflow configuration paths are not allowed: ${target}`,
      );
    }
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw error;
  }
}

export async function prepareWorkflowEsmScope(
  configDir: string,
  sdkPath: string,
): Promise<void> {
  const workflowsDir = path.join(configDir, "workflows");
  if (!(await existingRegularPath(workflowsDir))) return;

  const packagePath = path.join(configDir, "package.json");
  if (!(await existingRegularPath(packagePath))) {
    await writeFile(
      packagePath,
      JSON.stringify({ private: true, type: "module" }, null, 2),
      "utf-8",
    );
  }

  const tsconfigPath = path.join(workflowsDir, "tsconfig.json");
  const relativeSdkPath = path.relative(workflowsDir, sdkPath);
  const sdkReference = path.isAbsolute(relativeSdkPath)
    ? sdkPath.replaceAll(path.sep, "/")
    : relativeSdkPath.replaceAll(path.sep, "/");
  if (await existingRegularPath(tsconfigPath)) {
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
