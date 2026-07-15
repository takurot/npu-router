import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import {
  materializeFixtures,
  validateCatalog,
  verifyFixtures,
} from "../scripts/fetch-test-fixtures.mjs";

const helloSha256 =
  "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

function catalogFor(artifact) {
  return { schemaVersion: 1, artifacts: [artifact] };
}

test("accepts the repository fixture catalog", async () => {
  const catalog = JSON.parse(
    await readFile(new URL("../fixtures/catalog.json", import.meta.url), "utf8"),
  );

  assert.equal(validateCatalog(catalog).artifacts.length, 5);
});

test("rejects traversal and non-HTTPS download URLs", () => {
  assert.throws(
    () =>
      validateCatalog(
        catalogFor({
          id: "bad-path",
          target: "../outside",
          size: 5,
          sha256: helloSha256,
          source: { url: "https://raw.githubusercontent.com/example/file" },
        }),
      ),
    /safe relative path/,
  );

  assert.throws(
    () =>
      validateCatalog(
        catalogFor({
          id: "bad-url",
          target: "models/file.onnx",
          size: 5,
          sha256: helloSha256,
          source: { url: "http://example.com/file" },
        }),
      ),
    /approved HTTPS host/,
  );
});

test("materializes inline and downloaded artifacts after checksum verification", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const catalog = {
    schemaVersion: 1,
    artifacts: [
      {
        id: "inline",
        target: "golden/inline.bin",
        size: 5,
        sha256: helloSha256,
        source: { base64: Buffer.from("hello").toString("base64") },
      },
      {
        id: "download",
        target: "models/download.bin",
        size: 5,
        sha256: helloSha256,
        source: {
          url: "https://raw.githubusercontent.com/owner/repo/commit/file",
        },
      },
    ],
  };
  let fetchCalls = 0;
  const fetchImpl = async () => {
    fetchCalls += 1;
    return new Response("hello", {
      status: 200,
      headers: { "content-length": "5" },
    });
  };

  try {
    assert.deepEqual(await materializeFixtures(catalog, root, fetchImpl), {
      created: 2,
      reused: 0,
    });
    assert.equal(await readFile(path.join(root, "golden/inline.bin"), "utf8"), "hello");
    assert.equal(await readFile(path.join(root, "models/download.bin"), "utf8"), "hello");
    assert.deepEqual(await materializeFixtures(catalog, root, fetchImpl), {
      created: 0,
      reused: 2,
    });
    assert.equal(fetchCalls, 1);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("does not retain a download with unexpected content", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const catalog = catalogFor({
    id: "corrupt",
    target: "models/corrupt.bin",
    size: 5,
    sha256: helloSha256,
    source: { url: "https://raw.githubusercontent.com/owner/repo/commit/file" },
  });

  try {
    await assert.rejects(
      materializeFixtures(catalog, root, async () => new Response("wrong")),
      /checksum mismatch/,
    );
    await assert.rejects(readFile(path.join(root, "models/corrupt.bin")), {
      code: "ENOENT",
    });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("stops reading a download that exceeds its declared limit", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const catalog = catalogFor({
    id: "oversized",
    target: "models/oversized.bin",
    size: 5,
    sha256: helloSha256,
    source: { url: "https://raw.githubusercontent.com/owner/repo/commit/file" },
  });

  try {
    await assert.rejects(
      materializeFixtures(catalog, root, async () => new Response("123456")),
      /size mismatch/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects a cache directory symlink that escapes the fixture root", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const outside = await mkdtemp(path.join(tmpdir(), "npu-fixtures-outside-"));
  const catalog = catalogFor({
    id: "escape",
    target: "models/escape.bin",
    size: 5,
    sha256: helloSha256,
    source: { base64: "aGVsbG8=" },
  });

  try {
    await symlink(outside, path.join(root, "models"), "dir");
    await assert.rejects(materializeFixtures(catalog, root), /escapes fixture root/);
    await assert.rejects(readFile(path.join(outside, "escape.bin")), { code: "ENOENT" });
  } finally {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});

test("verify reports missing and corrupt fixtures without repairing them", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const catalog = {
    schemaVersion: 1,
    artifacts: [
      {
        id: "corrupt",
        target: "corrupt.bin",
        size: 5,
        sha256: helloSha256,
        source: { base64: "aGVsbG8=" },
      },
      {
        id: "missing",
        target: "missing.bin",
        size: 5,
        sha256: helloSha256,
        source: { base64: "aGVsbG8=" },
      },
    ],
  };

  try {
    await writeFile(path.join(root, "corrupt.bin"), "wrong");
    assert.deepEqual(await verifyFixtures(catalog, root), [
      "corrupt: checksum mismatch",
      "missing: missing",
    ]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
