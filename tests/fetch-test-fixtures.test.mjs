import assert from "node:assert/strict";
import { mkdir, mkdtemp, readdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
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

test("rejects a Windows drive-letter target even without a leading slash", () => {
  assert.throws(
    () =>
      validateCatalog(
        catalogFor({
          id: "drive-letter",
          target: "C:/Windows/win.ini",
          size: 5,
          sha256: helloSha256,
          source: { url: "https://raw.githubusercontent.com/example/file" },
        }),
      ),
    /safe relative path/,
  );
});

test("rejects duplicate targets that only differ by case", () => {
  assert.throws(
    () =>
      validateCatalog({
        schemaVersion: 1,
        artifacts: [
          {
            id: "first",
            target: "qnn/golden/input.raw",
            size: 5,
            sha256: helloSha256,
            source: { base64: Buffer.from("hello").toString("base64") },
          },
          {
            id: "second",
            target: "qnn/Golden/Input.raw",
            size: 5,
            sha256: helloSha256,
            source: { base64: Buffer.from("hello").toString("base64") },
          },
        ],
      }),
    /unique safe relative path/,
  );
});

test("rejects malformed catalog entries", () => {
  const base = {
    id: "artifact",
    target: "models/file.onnx",
    size: 5,
    sha256: helloSha256,
    source: { base64: Buffer.from("hello").toString("base64") },
  };

  assert.throws(
    () => validateCatalog(catalogFor({ ...base, size: -1 })),
    /non-negative integer/,
  );
  assert.throws(
    () => validateCatalog(catalogFor({ ...base, size: 5.5 })),
    /non-negative integer/,
  );
  assert.throws(
    () => validateCatalog(catalogFor({ ...base, sha256: "not-a-hash" })),
    /64 lowercase hexadecimal/,
  );
  assert.throws(
    () => validateCatalog(catalogFor({ ...base, source: null })),
    /source is required/,
  );
  assert.throws(
    () => validateCatalog(catalogFor({ ...base, source: {} })),
    /exactly one of url or base64/,
  );
  assert.throws(
    () =>
      validateCatalog(
        catalogFor({
          ...base,
          source: { url: "https://raw.githubusercontent.com/x", base64: "aGVsbG8=" },
        }),
      ),
    /exactly one of url or base64/,
  );
  assert.throws(
    () => validateCatalog(catalogFor({ ...base, source: { base64: "not base64!" } })),
    /canonical base64/,
  );
  assert.throws(
    () => validateCatalog(catalogFor({ ...base, source: { url: "::not a url::" } })),
    /approved HTTPS host/,
  );
  assert.throws(
    () =>
      validateCatalog({
        schemaVersion: 1,
        artifacts: [base, { ...base, target: "models/other.onnx" }],
      }),
    /unique/,
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

test("requests downloads with redirects disabled", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const catalog = catalogFor({
    id: "no-redirect",
    target: "models/no-redirect.bin",
    size: 5,
    sha256: helloSha256,
    source: { url: "https://raw.githubusercontent.com/owner/repo/commit/file" },
  });
  let observedOptions;

  try {
    await materializeFixtures(catalog, root, async (_url, options) => {
      observedOptions = options;
      return new Response("hello", { status: 200, headers: { "content-length": "5" } });
    });
    assert.equal(observedOptions.redirect, "error");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects a non-2xx download response", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const catalog = catalogFor({
    id: "not-found",
    target: "models/not-found.bin",
    size: 5,
    sha256: helloSha256,
    source: { url: "https://raw.githubusercontent.com/owner/repo/commit/file" },
  });

  try {
    await assert.rejects(
      materializeFixtures(catalog, root, async () => new Response("nope", { status: 404 })),
      /HTTP 404/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects a download whose declared content-length disagrees with the catalog", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const catalog = catalogFor({
    id: "wrong-length",
    target: "models/wrong-length.bin",
    size: 5,
    sha256: helloSha256,
    source: { url: "https://raw.githubusercontent.com/owner/repo/commit/file" },
  });

  try {
    await assert.rejects(
      materializeFixtures(
        catalog,
        root,
        async () => new Response("hello", { status: 200, headers: { "content-length": "999" } }),
      ),
      /size mismatch/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects a fixture root that resolves through a symlink", async () => {
  const parent = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const outside = await mkdtemp(path.join(tmpdir(), "npu-fixtures-outside-"));
  const root = path.join(parent, "cache");
  const catalog = catalogFor({
    id: "root-escape",
    target: "file.bin",
    size: 5,
    sha256: helloSha256,
    source: { base64: "aGVsbG8=" },
  });

  try {
    await symlink(outside, root, "dir");
    await assert.rejects(materializeFixtures(catalog, root), /resolves through a symlink/);
  } finally {
    await rm(parent, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});

test("does not create directories outside the fixture root before rejecting a symlinked parent", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const outside = await mkdtemp(path.join(tmpdir(), "npu-fixtures-outside-"));
  const catalog = catalogFor({
    id: "nested-escape",
    target: "models/nested/deep/escape.bin",
    size: 5,
    sha256: helloSha256,
    source: { base64: "aGVsbG8=" },
  });

  try {
    await symlink(outside, path.join(root, "models"), "dir");
    await assert.rejects(verifyFixtures(catalog, root), /escapes fixture root/);
    assert.deepEqual(await readdir(outside), []);
  } finally {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});

test("replaces a leaf symlink without trusting or leaking the file it points to", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "npu-fixtures-"));
  const outside = await mkdtemp(path.join(tmpdir(), "npu-fixtures-outside-"));
  const secretPath = path.join(outside, "secret.bin");
  await writeFile(secretPath, "hello");
  const catalog = catalogFor({
    id: "leaf-symlink",
    target: "models/leaf.bin",
    size: 5,
    sha256: helloSha256,
    source: { base64: Buffer.from("hello").toString("base64") },
  });

  try {
    await mkdir(path.join(root, "models"), { recursive: true });
    await symlink(secretPath, path.join(root, "models", "leaf.bin"));

    assert.deepEqual(await materializeFixtures(catalog, root), { created: 1, reused: 0 });
    assert.equal(await readFile(path.join(root, "models", "leaf.bin"), "utf8"), "hello");
    assert.equal(await readFile(secretPath, "utf8"), "hello");
  } finally {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
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
