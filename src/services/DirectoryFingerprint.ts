import { createHash } from "node:crypto";
import { lstat, readdir, readFile } from "node:fs/promises";
import path from "node:path";

async function collectRegularFiles(
  directory: string,
  relative = "",
): Promise<string[]> {
  const current = path.join(directory, relative);
  const entries = await readdir(current, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const child = path.join(relative, entry.name);
    if (entry.isSymbolicLink()) {
      throw new Error(
        `Symbolic links are not allowed in fingerprinted directories: ${path.join(directory, child)}`,
      );
    }
    if (entry.isDirectory()) {
      files.push(...(await collectRegularFiles(directory, child)));
    } else if (entry.isFile()) {
      files.push(child);
    } else {
      throw new Error(
        `Unsupported filesystem entry in fingerprinted directory: ${path.join(directory, child)}`,
      );
    }
  }

  return files;
}

export async function fingerprintDirectory(directory: string): Promise<string> {
  const files = await listRegularFiles(directory);
  const hash = createHash("sha256");

  for (const file of files) {
    hash.update(file.replaceAll(path.sep, "/"));
    hash.update("\0");
    hash.update(await readFile(path.join(directory, file)));
    hash.update("\0");
  }

  return hash.digest("hex");
}

export async function listRegularFiles(directory: string): Promise<string[]> {
  const root = await lstat(directory);
  if (root.isSymbolicLink()) {
    throw new Error(
      `Symbolic links are not allowed for fingerprinted directories: ${directory}`,
    );
  }
  if (!root.isDirectory()) {
    throw new Error(`Fingerprint target is not a directory: ${directory}`);
  }

  return (await collectRegularFiles(directory)).sort((left, right) =>
    left.localeCompare(right),
  );
}
