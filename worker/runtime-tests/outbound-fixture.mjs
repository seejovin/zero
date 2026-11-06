let mode = "ok";
let calls = [];
export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.hostname === "fixture-control.invalid") {
      if (url.pathname === "/reset") { mode = url.searchParams.get("mode") || "ok"; calls = []; }
      return Response.json({ mode, calls });
    }
    calls.push({ host: url.hostname, path: url.pathname, method: request.method, authorization_present: request.headers.has("authorization"), redirect: request.redirect });
    if (mode === "redirect") return new Response(null, { status: 307, headers: { Location: "https://forbidden-redirect.invalid/leak" } });
    if (request.method !== "POST") return Response.json({ items: [] });
    const body = await request.json();
    if (body.method !== "eth_chainId") return Response.json({ jsonrpc: "2.0", id: body.id, error: { code: -32601, message: "Unexpected fixture operation" } });
    return Response.json({ jsonrpc: "2.0", id: body.id, result: url.hostname.includes("base") ? "0x2105" : "0x1" });
  }
};
