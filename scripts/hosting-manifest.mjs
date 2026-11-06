import { lstat, readFile } from "node:fs/promises";
import { resolve } from "node:path";

/** Select build metadata without creating a Site or changing a deployment. */
export async function loadHostingManifest(root, environment = process.env) {
  const explicitPath = environment.ZERO_HOSTING_MANIFEST;
  if (explicitPath !== undefined && (typeof explicitPath !== "string" || explicitPath.trim() === "")) {
    throw new Error("ZERO_HOSTING_MANIFEST must name a JSON manifest file.");
  }
  let sourcePath = resolve(root, explicitPath ?? ".openai/hosting.json");
  if (explicitPath === undefined) {
    try {
      // A present but broken symlink is configured metadata, not a new checkout.
      await lstat(sourcePath);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      sourcePath = resolve(root, "config/hosting.example.json");
    }
  }
  // Parsing/read errors never fall back to a different target.
  const manifest = JSON.parse(await readFile(sourcePath, "utf8"));
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)
      || Object.hasOwn(manifest, "static")) {
    throw new Error("The hosted wallet requires a Worker manifest without a static field.");
  }
  if (Object.hasOwn(manifest, "project_id")
      && (typeof manifest.project_id !== "string" || manifest.project_id.length === 0
          || manifest.project_id.length > 256 || /\s/.test(manifest.project_id))) {
    throw new Error("A configured project_id must be a nonempty identifier.");
  }
  return { manifest, sourcePath, mode: Object.hasOwn(manifest, "project_id") ? "configured" : "portable" };
}
