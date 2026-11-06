import { parseStrictJson } from "../web/src/strict-json";

/** The relay never receives a seed, password, vault or unsigned signing request.
 * It transports a small allowlist of public chain queries and already-signed
 * transactions. Provider replies remain untrusted inputs to the Rust validator. */
type Chain = 1 | 8453;
type Fetcher = typeof fetch;
type ObjectValue = Record<string, unknown>;
export interface RelayEnvironment { BLOCKSCOUT_API_KEY?: string }

export const PROVIDERS: Record<Chain, readonly string[]> = {
  1: ["https://ethereum-rpc.publicnode.com", "https://eth-mainnet.g.alchemy.com/public"],
  8453: ["https://base-rpc.publicnode.com", "https://mainnet.base.org"],
};
const SUBMIT: Record<Chain, string> = {
  1: "https://ethereum-rpc.publicnode.com",
  8453: "https://mainnet-sequencer.base.org",
};
const TOKENS: Record<Chain, readonly string[]> = {
  1: ["0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "0x1abaea1f7c830bd89acc67ec4af516284b1bc33c"],
  8453: ["0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", "0x60a3e35cc302bfa44cb288bc5a4f316fdb1adb42"],
};
const ORACLE = "0x420000000000000000000000000000000000000f";
const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const MAX_REQUEST_BYTES = 16_384;
const MAX_RESPONSE_BYTES = 1_048_576;
const TIMEOUT_MS = 12_000;
const HASH = /^0x[0-9a-fA-F]{64}$/;
const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
const QUANTITY = /^0x(?:0|[1-9a-fA-F][0-9a-fA-F]*)$/;
const preference = new Map<Chain, string>();
const verified = new Map<string, number>();
const cooling = new Map<string, number>();
const inflightChainChecks = new Map<string, Promise<void>>();

type FailurePhase = "request" | "body" | "validate";
type FailureReason = "timeout" | "http_status" | "rate_limit" | "redirect" | "response_too_large" | "invalid_response" | "invalid_json" | "wrong_chain" | "rpc_unavailable" | "type_error" | "network_error";
class RelayFailure extends Error {
  constructor(readonly reason: FailureReason = "network_error", readonly phase: FailurePhase = "request", readonly httpStatus?: number, readonly rpcCode?: number) { super(reason); }
}
function classifyFailure(error: unknown, phase: FailurePhase = "request", aborted = false): RelayFailure {
  if (error instanceof RelayFailure) return error;
  return new RelayFailure(aborted ? "timeout" : error instanceof TypeError ? "type_error" : "network_error", phase);
}
function diagnostic(fields: Record<string, unknown>): void {
  // Diagnostics contain only fixed labels, approved provider hosts and numeric
  // metadata. Never include a request/response body, URL query or exception text.
  try { console.warn(JSON.stringify(fields)); } catch { /* Logging cannot change transaction or read behavior. */ }
}
function failureDiagnostic(event: "wallet_rpc_upstream_failure" | "wallet_activity_upstream_failure", id: Chain, method: string, endpoint: string, error: unknown, started: number, phase?: "chain_check"): void {
  const failure = classifyFailure(error);
  diagnostic({ event, chain_id: id, method, provider_host: new URL(endpoint).hostname, phase: phase ?? failure.phase, reason: failure.reason, http_status: failure.httpStatus, rpc_code: failure.rpcCode, elapsed_ms: Math.max(0, Date.now() - started) });
}
async function rejectHttpFailure(result: Response): Promise<void> {
  if (result.ok) return;
  // Workers supports manual/follow only. Manual mode plus rejecting 3xx keeps
  // redirects disabled without the synchronous TypeError caused by "error".
  const reason = result.status >= 300 && result.status < 400 ? "redirect" : result.status === 429 ? "rate_limit" : "http_status";
  try { await result.body?.cancel(); } catch { /* Preserve the original status. */ }
  throw new RelayFailure(reason, "request", result.status);
}

function object(value: unknown): value is ObjectValue { return value !== null && typeof value === "object" && !Array.isArray(value); }
function only(value: ObjectValue, fields: readonly string[]): boolean { return Object.keys(value).every(key => fields.includes(key)); }
function address(value: unknown): value is string { return typeof value === "string" && ADDRESS.test(value); }
function hash(value: unknown): value is string { return typeof value === "string" && HASH.test(value); }
function quantity(value: unknown): value is string { return typeof value === "string" && value.length <= 66 && QUANTITY.test(value); }
function chain(value: unknown): value is Chain { return value === 1 || value === 8453; }
function blockTag(value: unknown): boolean {
  return value === "latest" || value === "pending" || value === "safe" || value === "finalized" || quantity(value)
    || (object(value) && only(value, ["blockHash", "requireCanonical"]) && hash(value.blockHash) && value.requireCanonical === true);
}
function transaction(value: unknown, id: Chain): boolean {
  if (!object(value) || !only(value, ["from", "to", "value", "data", "gas", "maxFeePerGas", "maxPriorityFeePerGas", "gasPrice"])) return false;
  if (!address(value.to) || (value.from !== undefined && !address(value.from))) return false;
  for (const name of ["value", "gas", "maxFeePerGas", "maxPriorityFeePerGas", "gasPrice"]) {
    if (value[name] !== undefined && !quantity(value[name])) return false;
  }
  if (value.gas !== undefined && BigInt(value.gas as string) > 300_000n) return false;
  const data = value.data ?? "0x";
  if (data === "0x") return true;
  if (typeof data !== "string") return false;
  const to = value.to.toLowerCase();
  if (TOKENS[id].includes(to)) return /^0x(?:70a08231[0-9a-fA-F]{64}|313ce567|a9059cbb[0-9a-fA-F]{128})$/.test(data);
  return id === 8453 && to === ORACLE && /^0x(?:f1c7a58b|275aedd2)[0-9a-fA-F]{64}$/.test(data);
}
function logFilter(value: unknown, id: Chain): boolean {
  if (!object(value) || !only(value, ["address", "fromBlock", "toBlock", "topics"])) return false;
  const contracts = Array.isArray(value.address) ? value.address : [value.address];
  if (!contracts.length || contracts.length > 2 || !contracts.every(item => address(item) && TOKENS[id].includes(item.toLowerCase()))) return false;
  if (!quantity(value.fromBlock) || !quantity(value.toBlock)) return false;
  const start = BigInt(value.fromBlock), end = BigInt(value.toBlock);
  if (end < start || end - start >= 2_000n) return false;
  const topics = value.topics;
  if (!Array.isArray(topics) || topics.length < 2 || topics.length > 3 || topics[0] !== TRANSFER) return false;
  // At least one indexed wallet address is required: no chain-wide event feed.
  const indexed = topics.slice(1);
  return indexed.some(hash) && indexed.every(item => item === null || (hash(item) && /^0x0{24}[0-9a-fA-F]{40}$/.test(item)));
}
export function validRpc(method: unknown, params: unknown, id: Chain): boolean {
  if (typeof method !== "string" || !Array.isArray(params) || params.length > 2) return false;
  switch (method) {
    case "eth_chainId": case "eth_blockNumber": case "eth_maxPriorityFeePerGas": case "eth_gasPrice": return params.length === 0;
    case "eth_getBlockByNumber": return params.length === 2 && typeof params[0] === "string" && blockTag(params[0]) && (params[1] === false || (params[1] === true && quantity(params[0])));
    case "eth_getBlockByHash": return params.length === 2 && hash(params[0]) && params[1] === false;
    case "eth_getBalance": case "eth_getCode": case "eth_getTransactionCount": return params.length === 2 && address(params[0]) && blockTag(params[1]);
    case "eth_getTransactionByHash": case "eth_getTransactionReceipt": return params.length === 1 && hash(params[0]);
    case "eth_call": case "eth_estimateGas": return params.length === 2 && transaction(params[0], id) && blockTag(params[1]);
    case "eth_getLogs": return params.length === 1 && logFilter(params[0], id);
    case "eth_sendRawTransaction": return params.length === 1 && typeof params[0] === "string" && /^0x02[0-9a-fA-F]+$/.test(params[0]) && params[0].length <= 2_050 && params[0].length % 2 === 0;
    default: return false;
  }
}

export async function boundedText(body: ReadableStream<Uint8Array> | null, limit: number): Promise<string> {
  if (!body) throw new RelayFailure("invalid_response", "body");
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const part = await reader.read();
    if (part.done) break;
    size += part.value.byteLength;
    if (size > limit) { await reader.cancel(); throw new RelayFailure("response_too_large", "body"); }
    chunks.push(part.value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}
function response(data: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(data), { status, headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", ...extra } });
}
function rpcError(id: unknown, code: number, message: string, status: number): Response {
  return response({ jsonrpc: "2.0", id: typeof id === "number" ? id : null, error: { code, message } }, status);
}
async function upstream(endpoint: string, payload: ObjectValue, fetcher: Fetcher): Promise<{ message: ObjectValue; text: string }> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  let phase: FailurePhase = "request";
  try {
    const result = await fetcher(endpoint, { method: "POST", headers: { "Content-Type": "application/json", "Accept": "application/json" }, body: JSON.stringify(payload), redirect: "manual", signal: controller.signal });
    await rejectHttpFailure(result);
    phase = "body";
    const declared = result.headers.get("content-length");
    if (declared && !/^\d+$/.test(declared)) throw new RelayFailure("invalid_response", phase);
    if (declared && Number(declared) > MAX_RESPONSE_BYTES) throw new RelayFailure("response_too_large", phase);
    const text = await boundedText(result.body, MAX_RESPONSE_BYTES);
    phase = "validate";
    let message: unknown;
    try { message = parseStrictJson(text); } catch { throw new RelayFailure("invalid_json", phase); }
    if (!object(message) || message.jsonrpc !== "2.0" || message.id !== payload.id || Object.hasOwn(message, "result") === Object.hasOwn(message, "error")) throw new RelayFailure("invalid_response", phase);
    return { message, text };
  } catch (error) {
    throw classifyFailure(error, phase, controller.signal.aborted);
  } finally { clearTimeout(timer); }
}
async function verifyProvider(endpoint: string, id: Chain, fetcher: Fetcher): Promise<void> {
  if ((verified.get(endpoint) ?? 0) > Date.now()) return;
  const existing = inflightChainChecks.get(endpoint);
  if (existing) return existing;
  const check = (async () => {
    const { message } = await upstream(endpoint, { jsonrpc: "2.0", id: 0, method: "eth_chainId", params: [] }, fetcher);
    if (!quantity(message.result) || BigInt(message.result) !== BigInt(id)) throw new RelayFailure("wrong_chain", "validate");
    verified.set(endpoint, Date.now() + 60_000);
  })();
  inflightChainChecks.set(endpoint, check);
  try { await check; } finally { inflightChainChecks.delete(endpoint); }
}

export async function relayRpc(request: Request, fetcher: Fetcher = fetch): Promise<Response> {
  if (request.method !== "POST") return rpcError(null, -32600, "Use POST for wallet RPC.", 405);
  const url = new URL(request.url);
  if (request.headers.get("origin") && request.headers.get("origin") !== url.origin) return rpcError(null, -32600, "Cross-origin wallet requests are not accepted.", 403);
  if (request.headers.get("sec-fetch-site") === "cross-site") return rpcError(null, -32600, "Cross-site wallet requests are not accepted.", 403);
  if (!/^application\/json(?:\s*;|$)/i.test(request.headers.get("content-type") ?? "")) return rpcError(null, -32600, "A JSON request is required.", 415);
  let input: unknown;
  try { input = parseStrictJson(await boundedText(request.body, MAX_REQUEST_BYTES)); }
  catch { return rpcError(null, -32700, "The request is malformed or too large.", 400); }
  if (!object(input) || !only(input, ["chainId", "jsonrpc", "id", "method", "params"]) || !chain(input.chainId) || input.jsonrpc !== "2.0" || !Number.isSafeInteger(input.id) || (input.id as number) <= 0 || !validRpc(input.method, input.params, input.chainId)) return rpcError(object(input) ? input.id : null, -32602, "This request is outside the wallet's supported mainnet operations.", 400);
  const id = input.chainId;
  const payload = { jsonrpc: "2.0", id: input.id, method: input.method, params: input.params };
  const sending = input.method === "eth_sendRawTransaction";
  if (sending) {
    // Exactly one upstream submission. A thrown request may already have
    // reached the network; never automatically retry it at another provider.
    const started = Date.now();
    try {
      const result = await upstream(SUBMIT[id], payload, fetcher);
      return new Response(result.text, { headers: { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", "X-Wallet-Rpc-Provider": new URL(SUBMIT[id]).hostname } });
    } catch (error) {
      failureDiagnostic("wallet_rpc_upstream_failure", id, String(input.method), SUBMIT[id], error, started);
      return rpcError(input.id, -32099, "Submission outcome is unknown. Check the saved transaction hash before retrying.", 502);
    }
  }
  const candidates = [...PROVIDERS[id]].sort((a, b) => Number(b === preference.get(id)) - Number(a === preference.get(id)));
  const available = candidates.filter(endpoint => (cooling.get(endpoint) ?? 0) <= Date.now());
  // If both have failed recently, fail quickly instead of a retry storm.
  if (!available.length) {
    diagnostic({ event: "wallet_rpc_cooldown", chain_id: id, method: input.method, remaining_ms: Math.max(0, Math.min(...candidates.map(endpoint => cooling.get(endpoint) ?? 0)) - Date.now()) });
    return rpcError(input.id, -32098, "Network reads are temporarily unavailable. Retry shortly.", 503);
  }
  for (const endpoint of available) {
    const started = Date.now();
    let checkingChain = input.method !== "eth_chainId";
    try {
      // A chain-ID read is itself the verification. Avoid doubling the first
      // request on a cold connection, while still checking every replacement.
      if (input.method !== "eth_chainId") await verifyProvider(endpoint, id, fetcher);
      checkingChain = false;
      const result = await upstream(endpoint, payload, fetcher);
      if (input.method === "eth_chainId") {
        if (!quantity(result.message.result) || BigInt(result.message.result) !== BigInt(id)) throw new RelayFailure("wrong_chain", "validate");
        verified.set(endpoint, Date.now() + 60_000);
      }
      const error = result.message.error;
      if (object(error) && [-32603, -32005, -32016].includes(Number(error.code))) throw new RelayFailure("rpc_unavailable", "validate", undefined, Number(error.code));
      preference.set(id, endpoint);
      return new Response(result.text, { headers: { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", "X-Wallet-Rpc-Provider": new URL(endpoint).hostname } });
    } catch (error) {
      failureDiagnostic("wallet_rpc_upstream_failure", id, String(input.method), endpoint, error, started, checkingChain ? "chain_check" : undefined);
      verified.delete(endpoint);
      cooling.set(endpoint, Date.now() + 15_000);
    }
  }
  return rpcError(input.id, -32098, "Balances or network data could not be refreshed. No transaction was submitted by this read.", 503);
}

export async function activityCandidates(request: Request, env: RelayEnvironment, fetcher: Fetcher = fetch): Promise<Response> {
  if (request.method !== "GET") return response({ error: "Use GET for activity discovery." }, 405);
  const url = new URL(request.url);
  const id = Number(url.searchParams.get("chainId"));
  const account = url.searchParams.get("address");
  if (!chain(id) || !address(account) || [...url.searchParams.keys()].some(key => !["chainId", "address"].includes(key)) || url.searchParams.getAll("chainId").length !== 1 || url.searchParams.getAll("address").length !== 1) return response({ error: "An Ethereum or Base address is required." }, 400);
  const base = { chainId: id, address: account, source: "blockscout", hashes: [] as string[], partial: true };
  // Blockscout's current universal API requires a project API key. Its old
  // per-instance routes are deprecated; do not silently rely on them.
  if (!env.BLOCKSCOUT_API_KEY) return response({ ...base, notice: "Full indexed history is not connected. Recent chain transfers and transaction-hash checks remain available." });
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    const hashes = new Set<string>();
    const results = await Promise.allSettled(["token-transfers", "transactions"].map(async path => {
      const endpoint = `https://api.blockscout.com/${id}/api/v2/addresses/${account}/${path}?filter=to`;
      const started = Date.now();
      let phase: FailurePhase = "request";
      try {
        const result = await fetcher(endpoint, { headers: { "Accept": "application/json", "Authorization": `Bearer ${env.BLOCKSCOUT_API_KEY}` }, redirect: "manual", signal: controller.signal });
        await rejectHttpFailure(result);
        phase = "body";
        const text = await boundedText(result.body, MAX_RESPONSE_BYTES);
        phase = "validate";
        let data: unknown;
        try { data = parseStrictJson(text); } catch { throw new RelayFailure("invalid_json", phase); }
        if (!object(data) || !Array.isArray(data.items)) throw new RelayFailure("invalid_response", phase);
        for (const item of data.items.slice(0, 50)) {
          if (!object(item)) continue;
          const candidate = item.transaction_hash ?? item.hash;
          if (hash(candidate)) hashes.add(candidate.toLowerCase());
        }
      } catch (error) {
        const failure = classifyFailure(error, phase, controller.signal.aborted);
        failureDiagnostic("wallet_activity_upstream_failure", id, "GET", endpoint, failure, started);
        throw failure;
      }
    }));
    return response({ ...base, hashes: [...hashes].slice(0, 30), notice: results.some(result => result.status === "rejected") ? "Indexed activity is incomplete. Recent chain scans continue independently." : "Recent indexed candidates; receipt verification is required." });
  } catch { return response({ ...base, notice: "Indexed activity is unavailable. Recent chain scans and transaction-hash checks remain available." }); }
  finally { clearTimeout(timer); }
}

/** Tests use deterministic transport state between independent scenarios. */
export function resetRelayStateForTests(): void { preference.clear(); verified.clear(); cooling.clear(); inflightChainChecks.clear(); }
