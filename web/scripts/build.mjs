import { build } from "esbuild";
import { cp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = path.join(root, "dist");
const version = JSON.parse(await readFile(path.join(root, "package.json"), "utf8")).version;
const wasm = path.resolve(root, "../build/wasm");
// Fail before touching dist if the shared production cryptography is missing.
for (const file of ["wallet_core.js", "wallet_core_bg.wasm"]) await readFile(path.join(wasm, file));
await rm(output, { recursive: true, force: true });
await mkdir(path.join(output, "crypto"), { recursive: true });
await cp(path.join(root, "public"), output, { recursive: true });
for (const file of ["wallet_core.js", "wallet_core_bg.wasm"]) await cp(path.join(wasm, file), path.join(output, "crypto", file));
await build({
  entryPoints: { app: path.join(root, "src/main.ts"), worker: path.join(root, "src/core-worker.ts"), diagnostics: path.join(root, "src/diagnostics.ts") },
  outdir: output, bundle: true, format: "esm", platform: "browser", target: "es2022", minify: true,
  sourcemap: false, legalComments: "eof", charset: "utf8", external: ["./crypto/wallet_core.js"],
});
const files = ["app.js", "worker.js", "diagnostics.js", "styles.css", "crypto/wallet_core.js", "crypto/wallet_core_bg.wasm"];
const hashes = {};
for (const file of files) hashes[file] = createHash("sha256").update(await readFile(path.join(output, file))).digest("hex");
await writeFile(path.join(output, "build-info.json"), JSON.stringify({ version, networks: [1, 8453], hashes }, null, 2) + "\n");
const styleIntegrity = createHash("sha384").update(await readFile(path.join(output, "styles.css"))).digest("base64");
for (const [page, script] of [["index.html", "app.js"], ["diagnostics.html", "diagnostics.js"]]) {
  const scriptIntegrity = createHash("sha384").update(await readFile(path.join(output, script))).digest("base64");
  let html = await readFile(path.join(output, page), "utf8");
  html = html.replace(`src="./${script}"`, `src="./${script}" integrity="sha384-${scriptIntegrity}" crossorigin="anonymous"`)
    .replace('href="./styles.css"', `href="./styles.css" integrity="sha384-${styleIntegrity}" crossorigin="anonymous"`);
  await writeFile(path.join(output, page), html);
}
console.log(`Built Local Wallet web ${version}: ${output}`);
console.log(`Shared Rust WASM SHA-256: ${hashes["crypto/wallet_core_bg.wasm"]}`);
