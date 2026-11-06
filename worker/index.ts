import { relayRpc, activityCandidates, type RelayEnvironment } from "./relay";
// Generated from the validated local web build. No filesystem or dynamic URLs.
import { assets } from "./assets.generated";

const SECURITY = {
  "Content-Security-Policy": "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self' https://ethereum-rpc.publicnode.com https://eth-mainnet.g.alchemy.com https://base-rpc.publicnode.com https://mainnet.base.org https://mainnet-sequencer.base.org; worker-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'; frame-ancestors 'none'; manifest-src 'self'",
  "X-Content-Type-Options": "nosniff",
  "X-Frame-Options": "DENY",
  "Referrer-Policy": "no-referrer",
  "Permissions-Policy": "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=()",
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cache-Control": "no-store",
};
const decoded = new Map<string, Uint8Array<ArrayBuffer>>();
export default {
  async fetch(request: Request, env: RelayEnvironment = {}): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/api/rpc") return relayRpc(request);
    if (url.pathname === "/api/activity") return activityCandidates(request, env);
    if (url.pathname === "/api/health" && request.method === "GET") return new Response(JSON.stringify({ version: "0.6.0", relay: true, networks: [1, 8453] }), { headers: { ...SECURITY, "Content-Type": "application/json" } });
    if (!["GET", "HEAD"].includes(request.method)) return new Response("Method not allowed", { status: 405, headers: SECURITY });
    const path = url.pathname === "/" ? "/index.html" : url.pathname;
    const asset = assets[path];
    if (!asset) return new Response("Not found", { status: 404, headers: SECURITY });
    let bytes = decoded.get(path);
    if (!bytes) { bytes = Uint8Array.from(atob(asset.base64), character => character.charCodeAt(0)); decoded.set(path, bytes); }
    return new Response(request.method === "HEAD" ? null : bytes, { headers: { ...SECURITY, "Content-Type": asset.type, "Content-Length": String(bytes.byteLength) } });
  },
};
