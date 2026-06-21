import assert from "node:assert/strict";
import test from "node:test";

import {
  validateWorkspaceGraph,
  workspaceGraphFromMetadata,
} from "../scripts/check-crate-boundaries.mjs";

const expectedPackages = [
  "npu-core",
  "npu-ort",
  "npu-qnn",
  "npu-tasks",
  "npu-sdk",
  "npu-cli",
  "npu-server",
];

test("accepts the intended workspace dependency graph", () => {
  const dependencies = new Map([
    ["npu-core", []],
    ["npu-ort", ["npu-core"]],
    ["npu-qnn", ["npu-core", "npu-ort"]],
    ["npu-tasks", ["npu-core"]],
    ["npu-sdk", ["npu-core", "npu-ort", "npu-qnn", "npu-tasks"]],
    ["npu-cli", ["npu-sdk"]],
    ["npu-server", ["npu-sdk"]],
  ]);

  assert.deepEqual(validateWorkspaceGraph(expectedPackages, dependencies), []);
});

test("rejects a dependency direction violation", () => {
  const dependencies = new Map(expectedPackages.map((name) => [name, []]));
  dependencies.set("npu-core", ["npu-sdk"]);

  assert.deepEqual(validateWorkspaceGraph(expectedPackages, dependencies), [
    "forbidden workspace dependency: npu-core -> npu-sdk",
  ]);
});

test("rejects a workspace dependency cycle", () => {
  const dependencies = new Map(expectedPackages.map((name) => [name, []]));
  dependencies.set("npu-ort", ["npu-core"]);
  dependencies.set("npu-core", ["npu-ort"]);

  assert.deepEqual(validateWorkspaceGraph(expectedPackages, dependencies), [
    "forbidden workspace dependency: npu-core -> npu-ort",
    "workspace dependency cycle: npu-core -> npu-ort -> npu-core",
  ]);
});

test("rejects missing and unexpected workspace crates", () => {
  const packages = expectedPackages.filter((name) => name !== "npu-server");
  packages.push("unplanned-crate");
  const dependencies = new Map(packages.map((name) => [name, []]));

  assert.deepEqual(validateWorkspaceGraph(packages, dependencies), [
    "missing workspace crate: npu-server",
    "unexpected workspace crate: unplanned-crate",
  ]);
});

test("extracts only workspace dependencies from Cargo metadata", () => {
  const metadata = {
    workspace_members: ["path+npu-core", "path+npu-ort"],
    packages: [
      { id: "path+npu-core", name: "npu-core", dependencies: [] },
      {
        id: "path+npu-ort",
        name: "npu-ort",
        dependencies: [{ name: "npu-core" }, { name: "external" }],
      },
      { id: "registry+external", name: "external", dependencies: [] },
    ],
  };

  assert.deepEqual(workspaceGraphFromMetadata(metadata), {
    packages: ["npu-core", "npu-ort"],
    dependencies: new Map([
      ["npu-core", []],
      ["npu-ort", ["npu-core"]],
    ]),
  });
});
