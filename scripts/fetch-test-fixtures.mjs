import { createHash, randomUUID } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, lstat, mkdir, realpath, rename, rm, writeFile } from "node:fs/promises";
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
    !value.includes(":") &&
    !path.posix.isAbsolute(value) &&
    !path.win32.isAbsolute(value) &&
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
    const normalizedTarget =
      typeof artifact.target === "string" ? artifact.target.toLowerCase() : artifact.target;
    if (!isSafeRelativePath(artifact.target) || targets.has(normalizedTarget)) {
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
    targets.add(normalizedTarget);
  }
  return catalog;
}

function sha256Buffer(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function sha256File(target) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(target);
    stream.on("error", reject);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest("hex")));
  });
}

function validateBytes(artifact, bytes) {
  if (bytes.byteLength !== artifact.size) {
    throw new Error(
      `${artifact.id}: size mismatch (expected ${artifact.size}, received ${bytes.byteLength})`,
    );
  }
  if (sha256Buffer(bytes) !== artifact.sha256) {
    throw new Error(`${artifact.id}: checksum mismatch`);
  }
}

async function existingArtifactStatus(artifact, target) {
  let metadata;
  try {
    metadata = await lstat(target);
  } catch (error) {
    if (error?.code === "ENOENT") return "missing";
    throw error;
  }
  if (!metadata.isFile() || metadata.size !== artifact.size) return "checksum mismatch";
  return (await sha256File(target)) === artifact.sha256 ? "ok" : "checksum mismatch";
}

async function containedTarget(root, relativeTarget) {
  const resolvedRoot = path.resolve(root);
  await mkdir(resolvedRoot, { recursive: true });
  const realRoot = await realpath(resolvedRoot);
  if (realRoot !== resolvedRoot) {
    throw new Error("fixture root resolves through a symlink");
  }

  const segments = relativeTarget.split("/");
  const fileName = segments.pop();
  let current = realRoot;
  for (const segment of segments) {
    const next = path.join(current, segment);
    let stats;
    try {
      stats = await lstat(next);
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
      await mkdir(next);
      current = next;
      continue;
    }
    if (!stats.isDirectory()) {
      throw new Error(`${relativeTarget}: parent directory escapes fixture root`);
    }
    current = next;
  }
  return path.join(current, fileName);
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
