import { spawnSync } from "node:child_process";
import { copyFile, mkdtemp, rm } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { tmpdir } from "node:os";

const [runtime, bundle] = process.argv.slice(2);
if (!runtime || !bundle || !isAbsolute(runtime) || !isAbsolute(bundle) || process.argv.length !== 4) {
  throw new Error("Usage: node verify.mjs /absolute/path/to/workerd /absolute/path/to/server/index.js");
}
const version = spawnSync(runtime, ["--version"], { encoding: "utf8", timeout: 10_000 });
if (version.status !== 0 || version.stdout.trim() !== "workerd 2026-11-04") {
  throw new Error("Use the pinned official workerd@1.20261104.1 runtime for this verification.");
}
const folder = await mkdtemp(join(tmpdir(), "local-wallet-workerd-"));
try {
  await copyFile(bundle, join(folder, "worker-under-test.mjs"));
  for (const name of ["acceptance.test.mjs", "outbound-fixture.mjs", "runtime.capnp"]) {
    await copyFile(new URL(name, import.meta.url), join(folder, name));
  }
  const result = spawnSync(runtime, ["test", join(folder, "runtime.capnp")], { cwd: folder, stdio: "inherit", timeout: 60_000 });
  if (result.error) throw result.error;
  if (result.signal) throw new Error(`Native Worker verification terminated: ${result.signal}`);
  process.exitCode = result.status ?? 1;
} finally {
  await rm(folder, { recursive: true, force: true });
}
