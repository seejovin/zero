import { readFile, writeFile, mkdir, rm } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "../web/node_modules/esbuild/lib/main.js";
import { loadHostingManifest } from "./hosting-manifest.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const { manifest, mode } = await loadHostingManifest(root);
const web = resolve(root, "web/dist");
const files = {
  "index.html": "text/html; charset=utf-8",
  "diagnostics.html": "text/html; charset=utf-8",
  "app.js": "text/javascript; charset=utf-8",
  "diagnostics.js": "text/javascript; charset=utf-8",
  "worker.js": "text/javascript; charset=utf-8",
  "styles.css": "text/css; charset=utf-8",
  "favicon.svg": "image/svg+xml",
  "crypto/wallet_core.js": "text/javascript; charset=utf-8",
  "crypto/wallet_core_bg.wasm": "application/wasm",
  "build-info.json": "application/json; charset=utf-8",
};
const assets = {};
for (const [file, type] of Object.entries(files)) {
  let content = await readFile(resolve(web, file));
  if (file === "index.html" || file === "diagnostics.html") {
    let html = content.toString("utf8");
    html = html.replace("<head>", '<head>\n  <meta name="wallet-rpc-relay" content="/api/rpc">');
    content = Buffer.from(html);
  }
  assets[`/${file}`] = { type, base64: content.toString("base64") };
}
await writeFile(resolve(root, "worker/assets.generated.ts"), "export const assets: Record<string, {type:string;base64:string}> = " + JSON.stringify(assets) + ";\n");
await rm(resolve(root, "dist"), { recursive: true, force: true });
await mkdir(resolve(root, "dist/server"), { recursive: true });
await mkdir(resolve(root, "dist/.openai"), { recursive: true });
await build({ entryPoints: [resolve(root, "worker/index.ts")], outfile: resolve(root, "dist/server/index.js"), bundle: true, format: "esm", platform: "neutral", target: "es2022", minify: true, sourcemap: false });
await writeFile(resolve(root, "dist/.openai/hosting.json"), JSON.stringify(manifest, null, 2) + "\n");
console.log(`Built wallet Worker with embedded static assets and bounded mainnet RPC relay (${mode} hosting metadata).`);
