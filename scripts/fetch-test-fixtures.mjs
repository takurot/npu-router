import { createHash, randomUUID } from "node:crypto";
import { readFile, mkdir, realpath, rename, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const approvedDownloadHosts = new Set([
  "media.githubusercontent.com",
  "raw.githubusercontent.com",
]);
const sha256Pattern = /^[a-f0-9]{64}$/;
const base64Pattern = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;

function isSafeRelativePath(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value === value.replaceAll("\\", "/") &&
    !path.posix.isAbsolute(value) &&
    path.posix.normalize(value) === value &&
    !value.split("/").includes("..")
  );
}

function decodeSource(artifact) {
  if ("base64" in artifact.source) {
    return Buffer.from(artifact.source.base64, "base64");
  }
  return undefined;
}

export function validateCatalog(catalog) {
  if (catalog?.schemaVersion !== 1 || !Array.isArray(catalog.artifacts)) {
    throw new Error("fixture catalog must use schemaVersion 1 and contain artifacts");
  }

  const ids = new Set();
  const targets = new Set();
  for (const artifact of catalog.artifacts) {
    if (typeof artifact.id !== "string" || artifact.id.length === 0 || ids.has(artifact.id)) {
      throw new Error("fixture artifact IDs must be non-empty and unique");
    }
    if (!isSafeRelativePath(artifact.target) || targets.has(artifact.target)) {
      throw new Error(`${artifact.id}: target must be a unique safe relative path`);
    }
    if (!Number.isSafeInteger(artifact.size) || artifact.size < 0) {
      throw new Error(`${artifact.id}: size must be a non-negative integer`);
    }
    if (!sha256Pattern.test(artifact.sha256)) {
      throw new Error(`${artifact.id}: sha256 must be 64 lowercase hexadecimal characters`);
    }
    if (artifact.source === null || typeof artifact.source !== "object") {
      throw new Error(`${artifact.id}: source is required`);
    }

    const hasUrl = Object.hasOwn(artifact.source, "url");
    const hasBase64 = Object.hasOwn(artifact.source, "base64");
    if (hasUrl === hasBase64) {
      throw new Error(`${artifact.id}: source must contain exactly one of url or base64`);
    }
    if (hasUrl) {
      let url;
      try {
        url = new URL(artifact.source.url);
      } catch {
        throw new Error(`${artifact.id}: download URL must use an approved HTTPS host`);
      }
      if (url.protocol !== "https:" || !approvedDownloadHosts.has(url.hostname)) {
        throw new Error(`${artifact.id}: download URL must use an approved HTTPS host`);
      }
    } else if (
      typeof artifact.source.base64 !== "string" ||
      !base64Pattern.test(artifact.source.base64)
    ) {
      throw new Error(`${artifact.id}: inline source must be canonical base64`);
    }

    ids.add(artifact.id);
    targets.add(artifact.target);
  }
  return catalog;
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function validateBytes(artifact, bytes) {
  if (bytes.byteLength !== artifact.size) {
    throw new Error(
      `${artifact.id}: size mismatch (expected ${artifact.size}, received ${bytes.byteLength})`,
    );
  }
  if (sha256(bytes) !== artifact.sha256) {
    throw new Error(`${artifact.id}: checksum mismatch`);
  }
}

async function existingArtifactStatus(artifact, target) {
  try {
    const metadata = await stat(target);
    if (!metadata.isFile() || metadata.size !== artifact.size) return "checksum mismatch";
    return sha256(await readFile(target)) === artifact.sha256 ? "ok" : "checksum mismatch";
  } catch (error) {
    if (error?.code === "ENOENT") return "missing";
    throw error;
  }
}

async function containedTarget(root, relativeTarget) {
  const resolvedRoot = path.resolve(root);
  await mkdir(resolvedRoot, { recursive: true });
  const realRoot = await realpath(resolvedRoot);
  if (realRoot !== resolvedRoot) {
    throw new Error("fixture root resolves through a symlink");
  }

  const target = path.join(resolvedRoot, ...relativeTarget.split("/"));
  const parent = path.dirname(target);
  await mkdir(parent, { recursive: true });
  const realParent = await realpath(parent);
  const relativeParent = path.relative(realRoot, realParent);
  if (relativeParent === ".." || relativeParent.startsWith(`..${path.sep}`)) {
    throw new Error(`${relativeTarget}: parent directory escapes fixture root`);
  }
  return path.join(realParent, path.basename(target));
}

async function fetchArtifact(artifact, fetchImpl) {
  const response = await fetchImpl(artifact.source.url, { redirect: "error" });
  if (!response.ok) {
    throw new Error(`${artifact.id}: download failed with HTTP ${response.status}`);
  }
  const declaredSize = response.headers.get("content-length");
  if (declaredSize !== null && Number(declaredSize) !== artifact.size) {
    throw new Error(`${artifact.id}: size mismatch (expected ${artifact.size}, received ${declaredSize})`);
  }
  if (response.body === null) throw new Error(`${artifact.id}: download returned no body`);

  const chunks = [];
  const reader = response.body.getReader();
  let received = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    received += value.byteLength;
    if (received > artifact.size) {
      await reader.cancel();
      throw new Error(`${artifact.id}: size mismatch (expected ${artifact.size}, received more)`);
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks, received);
}

export async function materializeFixtures(catalog, root, fetchImpl = fetch) {
  validateCatalog(catalog);
  let created = 0;
  let reused = 0;

  for (const artifact of catalog.artifacts) {
    const target = await containedTarget(root, artifact.target);
    if ((await existingArtifactStatus(artifact, target)) === "ok") {
      reused += 1;
      continue;
    }

    const bytes = decodeSource(artifact) ?? (await fetchArtifact(artifact, fetchImpl));
    validateBytes(artifact, bytes);
    await mkdir(path.dirname(target), { recursive: true });
    const temporary = `${target}.${randomUUID()}.tmp`;
    try {
      await writeFile(temporary, bytes, { flag: "wx", mode: 0o600 });
      await rename(temporary, target);
    } finally {
      await rm(temporary, { force: true });
    }
    created += 1;
  }
  return { created, reused };
}

export async function verifyFixtures(catalog, root) {
  validateCatalog(catalog);
  const errors = [];
  for (const artifact of catalog.artifacts) {
    const target = await containedTarget(root, artifact.target);
    const status = await existingArtifactStatus(artifact, target);
    if (status !== "ok") errors.push(`${artifact.id}: ${status}`);
  }
  return errors;
}

async function run() {
  const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const catalogPath = path.join(repositoryRoot, "fixtures/catalog.json");
  const fixtureRoot = path.join(repositoryRoot, ".cache/test-fixtures");
  const catalog = JSON.parse(await readFile(catalogPath, "utf8"));

  if (process.argv.slice(2).includes("--verify-only")) {
    const errors = await verifyFixtures(catalog, fixtureRoot);
    if (errors.length > 0) throw new Error(errors.join("\n"));
    console.log(`fixtures verified: ${catalog.artifacts.length}`);
    return;
  }

  const result = await materializeFixtures(catalog, fixtureRoot);
  console.log(`fixtures ready: ${result.created} created, ${result.reused} reused`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  run().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
