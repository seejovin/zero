import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { loadHostingManifest } from "./hosting-manifest.mjs";

async function workspace(t) {
  const root = await mkdtemp(join(tmpdir(), "zero-hosting-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, "config"));
  await writeFile(join(root, "config/hosting.example.json"), '{"d1":null,"r2":null}\n');
  return root;
}

test("a clean source checkout builds without a deployment identity", async t => {
  const selected = await loadHostingManifest(await workspace(t), {});
  assert.equal(selected.mode, "portable");
  assert.deepEqual(selected.manifest, { d1: null, r2: null });
});

test("the default preserves an existing deployment and never rewrites its file", async t => {
  const root = await workspace(t);
  await mkdir(join(root, ".openai"));
  const original = '{\n  "project_id": "appgprj_test_existing", "d1": null, "r2": null\n}\n';
  await writeFile(join(root, ".openai/hosting.json"), original);
  const selected = await loadHostingManifest(root, {});
  assert.equal(selected.mode, "configured");
  assert.equal(selected.manifest.project_id, "appgprj_test_existing");
  assert.equal(await readFile(join(root, ".openai/hosting.json"), "utf8"), original);
});

test("an explicit relative or absolute manifest is selected unchanged", async t => {
  const root = await workspace(t);
  const custom = { project_id: "appgprj_test_explicit", d1: null, r2: null };
  await writeFile(join(root, "custom.json"), JSON.stringify(custom));
  for (const path of ["custom.json", join(root, "custom.json")]) {
    assert.deepEqual((await loadHostingManifest(root, { ZERO_HOSTING_MANIFEST: path })).manifest, custom);
  }
});

test("a malformed existing manifest does not fall back to the template", async t => {
  const root = await workspace(t);
  await mkdir(join(root, ".openai"));
  await writeFile(join(root, ".openai/hosting.json"), "{");
  await assert.rejects(loadHostingManifest(root, {}), SyntaxError);
});

test("a missing explicit file and an empty explicit path both fail", async t => {
  const root = await workspace(t);
  await assert.rejects(loadHostingManifest(root, { ZERO_HOSTING_MANIFEST: "missing.json" }), { code: "ENOENT" });
  await assert.rejects(loadHostingManifest(root, { ZERO_HOSTING_MANIFEST: "" }), /must name/);
});

test("static deployment metadata and invalid project identifiers are rejected", async t => {
  const root = await workspace(t);
  for (const manifest of [{ static: { directory: "dist" } }, { static: null }, { project_id: "" }, { project_id: 1 }, { project_id: "a b" }, null, []]) {
    await writeFile(join(root, "bad.json"), JSON.stringify(manifest));
    await assert.rejects(loadHostingManifest(root, { ZERO_HOSTING_MANIFEST: "bad.json" }));
  }
});
