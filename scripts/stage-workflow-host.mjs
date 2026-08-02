import {
  cpSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
} from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const workspaceDirectory = path.dirname(
  path.dirname(fileURLToPath(import.meta.url)),
);
const profileDirectoryArgument = process.argv[2];

if (!profileDirectoryArgument) {
  throw new Error("Expected the Cargo profile directory as the first argument.");
}

const profileDirectory = path.resolve(
  workspaceDirectory,
  profileDirectoryArgument,
);
const sourceDirectory = path.join(workspaceDirectory, "workflow-host");
const destinationDirectory = path.join(profileDirectory, "workflow-host");

rmSync(destinationDirectory, { recursive: true, force: true });
mkdirSync(destinationDirectory, { recursive: true });
copy(path.join(sourceDirectory, "dist"), path.join(destinationDirectory, "dist"));
copy(
  path.join(sourceDirectory, "package.json"),
  path.join(destinationDirectory, "package.json"),
);

const sourcePackageJsonPath = path.join(sourceDirectory, "package.json");
const sourcePackage = readPackage(sourcePackageJsonPath);
const destinationNodeModules = path.join(destinationDirectory, "node_modules");

for (const dependencyName of Object.keys(sourcePackage.dependencies ?? {})) {
  stageDependency(dependencyName, sourcePackageJsonPath, destinationNodeModules);
}

function stageDependency(dependencyName, parentPackageJsonPath, nodeModulesPath) {
  const parentRequire = createRequire(parentPackageJsonPath);
  const dependencyPackageJsonPath = realpathSync(
    parentRequire.resolve(`${dependencyName}/package.json`),
  );
  const dependencyDirectory = path.dirname(dependencyPackageJsonPath);
  const destinationDependencyDirectory = path.join(
    nodeModulesPath,
    dependencyName,
  );

  copyPackage(dependencyDirectory, destinationDependencyDirectory);

  const dependencyPackage = readPackage(dependencyPackageJsonPath);
  const nestedNodeModules = path.join(
    destinationDependencyDirectory,
    "node_modules",
  );

  for (const nestedDependencyName of Object.keys(
    dependencyPackage.dependencies ?? {},
  )) {
    stageDependency(
      nestedDependencyName,
      dependencyPackageJsonPath,
      nestedNodeModules,
    );
  }

  for (const optionalDependencyName of Object.keys(
    dependencyPackage.optionalDependencies ?? {},
  )) {
    try {
      stageDependency(
        optionalDependencyName,
        dependencyPackageJsonPath,
        nestedNodeModules,
      );
    } catch (error) {
      if (!isMissingModule(error)) {
        throw error;
      }
    }
  }
}

function copy(source, destination) {
  mkdirSync(path.dirname(destination), { recursive: true });
  cpSync(source, destination, { recursive: true, dereference: true });
}

function copyPackage(source, destination) {
  mkdirSync(path.dirname(destination), { recursive: true });
  cpSync(source, destination, {
    recursive: true,
    dereference: true,
    filter: (currentSource) => {
      const relativePath = path.relative(source, currentSource);
      return !relativePath.split(path.sep).includes("node_modules");
    },
  });
}

function readPackage(packageJsonPath) {
  return JSON.parse(readFileSync(packageJsonPath, "utf8"));
}

function isMissingModule(error) {
  return (
    error instanceof Error &&
    ("code" in error &&
      (error.code === "MODULE_NOT_FOUND" ||
        error.code === "ERR_PACKAGE_PATH_NOT_EXPORTED"))
  );
}
