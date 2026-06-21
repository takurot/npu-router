import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const allowedDependencies = new Map([
  ["npu-core", new Set()],
  ["npu-ort", new Set(["npu-core"])],
  ["npu-qnn", new Set(["npu-core", "npu-ort"])],
  ["npu-tasks", new Set(["npu-core"])],
  ["npu-sdk", new Set(["npu-core", "npu-ort", "npu-qnn", "npu-tasks"])],
  ["npu-cli", new Set(["npu-sdk"])],
  ["npu-server", new Set(["npu-sdk"])],
]);

function findCycles(packages, dependencies) {
  const cycles = [];
  const visited = new Set();
  const active = [];

  function visit(name) {
    const activeIndex = active.indexOf(name);
    if (activeIndex !== -1) {
      cycles.push([...active.slice(activeIndex), name].join(" -> "));
      return;
    }
    if (visited.has(name)) return;

    active.push(name);
    for (const dependency of dependencies.get(name) ?? []) {
      if (packages.includes(dependency)) visit(dependency);
    }
    active.pop();
    visited.add(name);
  }

  for (const name of packages) visit(name);
  return [...new Set(cycles)];
}

export function validateWorkspaceGraph(packages, dependencies) {
  const errors = [];
  const expectedPackages = [...allowedDependencies.keys()];

  for (const name of expectedPackages) {
    if (!packages.includes(name)) errors.push(`missing workspace crate: ${name}`);
  }
  for (const name of packages) {
    if (!allowedDependencies.has(name)) errors.push(`unexpected workspace crate: ${name}`);
  }
  for (const name of packages) {
    const allowed = allowedDependencies.get(name) ?? new Set();
    for (const dependency of dependencies.get(name) ?? []) {
      if (packages.includes(dependency) && !allowed.has(dependency)) {
        errors.push(`forbidden workspace dependency: ${name} -> ${dependency}`);
      }
    }
  }
  for (const cycle of findCycles(packages, dependencies)) {
    errors.push(`workspace dependency cycle: ${cycle}`);
  }

  return errors;
}

export function workspaceGraphFromMetadata(metadata) {
  const workspaceIds = new Set(metadata.workspace_members);
  const workspacePackages = metadata.packages.filter(({ id }) => workspaceIds.has(id));
  const packages = workspacePackages.map(({ name }) => name);
  const packageNames = new Set(packages);
  const dependencies = new Map(
    workspacePackages.map((pkg) => [
      pkg.name,
      pkg.dependencies
        .map(({ name }) => name)
        .filter((name) => packageNames.has(name)),
    ]),
  );
  return { packages, dependencies };
}

function checkCargoMetadata() {
  const output = execFileSync(
    "cargo",
    ["metadata", "--locked", "--no-deps", "--format-version", "1"],
    { encoding: "utf8" },
  );
  const { packages, dependencies } = workspaceGraphFromMetadata(JSON.parse(output));
  const errors = validateWorkspaceGraph(packages, dependencies);
  if (errors.length > 0) {
    throw new Error(errors.join("\n"));
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    checkCargoMetadata();
    console.log("crate boundaries: ok");
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
