import test, { beforeEach } from "node:test";
import assert from "node:assert/strict";
import { relayRpc, activityCandidates, validRpc, PROVIDERS, resetRelayStateForTests } from "./relay";

const ORIGIN = "https://wallet.example";
const ACCOUNT = "0x1234567890123456789012345678901234567890";
const TOKEN = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const ACCOUNT_TOPIC = `0x${ACCOUNT.slice(2).padStart(64, "0")}`;
function request(method = "eth_chainId", params: unknown[] = [], chainId = 8453): Request {
  return new Request(`${ORIGIN}/api/rpc`, { method: "POST", headers: { "Content-Type": "application/json", Origin: ORIGIN }, body: JSON.stringify({ chainId, jsonrpc: "2.0", id: 4, method, params }) });
}
function result(payload: Record<string, unknown>, value: unknown): Response { return new Response(JSON.stringify({ jsonrpc: "2.0", id: payload.id, result: value })); }
function payload(options?: RequestInit): Record<string, unknown> { return JSON.parse(String(options?.body)); }
beforeEach(resetRelayStateForTests);

test("transport failover verifies the replacement chain and preserves the public request", async () => {
  const calls: { url: string; body: Record<string, unknown> }[] = [];
  const transport: typeof fetch = async (input, options) => {
    const body = payload(options); calls.push({ url: String(input), body });
    if (String(input) === PROVIDERS[8453][0]) throw new TypeError("network unavailable");
    return result(body, body.method === "eth_chainId" ? "0x2105" : "0x186a0");
  };
  const response = await relayRpc(request("eth_getBalance", [ACCOUNT, "latest"]), transport);
  assert.equal(response.status, 200);
  assert.equal((await response.json()).result, "0x186a0");
  assert.deepEqual(calls.map(call => call.body.method), ["eth_chainId", "eth_chainId", "eth_getBalance"]);
  assert.deepEqual(calls[2].body.params, [ACCOUNT, "latest"]);
});

test("a provider reporting the wrong chain is not used for balances", async () => {
  const calls: { url: string; method: unknown }[] = [];
  const response = await relayRpc(request("eth_getBalance", [ACCOUNT, "latest"]), async (input, options) => {
    const body = payload(options); calls.push({ url: String(input), method: body.method });
    return result(body, String(input) === PROVIDERS[8453][0] ? "0x1" : body.method === "eth_chainId" ? "0x2105" : "0x7");
  });
  assert.equal((await response.json()).result, "0x7");
  assert.equal(calls.some(call => call.url === PROVIDERS[8453][0] && call.method === "eth_getBalance"), false);
});

test("raw submission timeout is one attempt and stays unknown", async () => {
  let calls = 0;
  const raw = "0x02abcd";
  const response = await relayRpc(request("eth_sendRawTransaction", [raw]), async (input, options) => {
    calls++;
    assert.equal(String(input), "https://mainnet-sequencer.base.org");
    assert.deepEqual(payload(options).params, [raw]);
    throw new Error("response lost after submission");
  });
  assert.equal(calls, 1);
  assert.equal(response.status, 502);
  assert.equal((await response.json()).error.code, -32099);
});

test("malformed, oversized, arbitrary method, testnet and secret-bearing payloads never reach a provider", async () => {
  let calls = 0;
  const transport: typeof fetch = async () => { calls++; throw new Error("must not be called"); };
  const body = { chainId: 8453, jsonrpc: "2.0", id: 4, method: "eth_chainId", params: [] };
  for (const invalid of [
    JSON.stringify({ ...body, chainId: 84532 }),
    JSON.stringify({ ...body, method: "eth_sign", params: [ACCOUNT, "0xabcd"] }),
    JSON.stringify({ ...body, seed: "never relay secrets" }),
    '{"chainId":8453,"chainId":1,"id":4,"jsonrpc":"2.0","method":"eth_chainId","params":[]}',
    " ".repeat(16_385),
  ]) {
    const response = await relayRpc(new Request(`${ORIGIN}/api/rpc`, { method: "POST", headers: { "Content-Type": "application/json" }, body: invalid }), transport);
    assert.equal(response.status, 400);
  }
  assert.equal(calls, 0);
});

test("cross-origin JSON request is rejected without dispatch", async () => {
  let calls = 0;
  const original = request();
  const response = await relayRpc(new Request(original, { headers: { "Content-Type": "application/json", Origin: "https://elsewhere.example" } }), async () => { calls++; throw new Error(); });
  assert.equal(response.status, 403);
  assert.equal(calls, 0);
});

test("duplicate provider fields cannot be normalized into plausible evidence", async () => {
  const response = await relayRpc(request(), async (_input, options) => {
    const body = payload(options);
    return new Response(`{"jsonrpc":"2.0","id":${body.id},"result":"0x2105","result":"0x1"}`);
  });
  assert.equal(response.status, 503);
});

test("RPC execution rejection is preserved and never retried into apparent success", async () => {
  let calls = 0;
  const response = await relayRpc(request("eth_call", [{ from: ACCOUNT, to: TOKEN, data: `0xa9059cbb${ACCOUNT_TOPIC.slice(2)}${"1".padStart(64, "0")}` }, "pending"]), async (_input, options) => {
    calls++;
    const body = payload(options);
    return body.method === "eth_chainId" ? result(body, "0x2105") : new Response(JSON.stringify({ jsonrpc: "2.0", id: body.id, error: { code: 3, message: "execution reverted" } }));
  });
  assert.equal((await response.json()).error.code, 3);
  assert.equal(calls, 2);
});

test("upstream response length is bounded before parsing", async () => {
  const response = await relayRpc(request(), async () => new Response("{}", { headers: { "Content-Length": "1048577" } }));
  assert.equal(response.status, 503);
});

test("log queries require allowlisted tokens, a participant and bounded heights", () => {
  const filter = { address: [TOKEN], fromBlock: "0x100", toBlock: "0x200", topics: [TRANSFER, null, ACCOUNT_TOPIC] };
  assert.equal(validRpc("eth_getLogs", [filter], 8453), true);
  assert.equal(validRpc("eth_getLogs", [{ ...filter, toBlock: "0x10000" }], 8453), false);
  assert.equal(validRpc("eth_getLogs", [{ ...filter, address: [ACCOUNT] }], 8453), false);
  assert.equal(validRpc("eth_getLogs", [{ ...filter, topics: [TRANSFER, null, null] }], 8453), false);
  assert.equal(validRpc("eth_getLogs", [{ ...filter, fromBlock: "latest" }], 8453), false);
});

test("arbitrary calldata and state overrides cannot pass the relay", () => {
  assert.equal(validRpc("eth_call", [{ to: TOKEN, data: "0x313ce567" }, "latest"], 8453), true);
  assert.equal(validRpc("eth_call", [{ to: TOKEN, data: "0x095ea7b3" + "0".repeat(128) }, "latest"], 8453), false);
  assert.equal(validRpc("eth_call", [{ to: ACCOUNT, data: "0xdeadbeef" }, "latest"], 8453), false);
  assert.equal(validRpc("eth_call", [{ to: TOKEN, data: "0x313ce567" }, "latest", {}], 8453), false);
});

test("unconfigured indexer reports limited coverage without pretending no deposit exists", async () => {
  let calls = 0;
  const response = await activityCandidates(new Request(`${ORIGIN}/api/activity?chainId=8453&address=${ACCOUNT}`), {}, async () => { calls++; throw new Error(); });
  const data = await response.json();
  assert.equal(data.partial, true);
  assert.deepEqual(data.hashes, []);
  assert.match(data.notice, /Recent chain transfers/);
  assert.equal(calls, 0);
});

test("unavailable reads cool down instead of exhausting providers on every refresh", async () => {
  let calls = 0;
  const transport: typeof fetch = async () => { calls++; throw new Error(); };
  assert.equal((await relayRpc(request(), transport)).status, 503);
  assert.equal((await relayRpc(request(), transport)).status, 503);
  assert.equal(calls, 2);
});

test("RPC and configured index fetches use the redirect mode supported by Workers", async () => {
  const modes: RequestRedirect[] = [];
  const transport: typeof fetch = async (input, options) => {
    // Node accepts redirect:error; Workers rejects it before network I/O. Keep
    // this boundary strict so a Node-only test cannot silently accept it again.
    assert.equal(options?.redirect, "manual");
    modes.push(options!.redirect!);
    return String(input).startsWith("https://api.blockscout.com/")
      ? new Response(JSON.stringify({ items: [] }))
      : result(payload(options), "0x2105");
  };
  const rpc = await relayRpc(request(), transport);
  assert.equal(rpc.status, 200);
  assert.equal((await rpc.json()).result, "0x2105");
  const activity = await activityCandidates(new Request(`${ORIGIN}/api/activity?chainId=8453&address=${ACCOUNT}`), { BLOCKSCOUT_API_KEY: "test-fixture-key" }, transport);
  assert.equal(activity.status, 200);
  assert.deepEqual((await activity.json()).hashes, []);
  assert.deepEqual(modes, ["manual", "manual", "manual"]);
});

test("a redirected read never follows Location and can use only the approved replacement provider", async () => {
  const calls: { url: string; method: unknown }[] = [];
  const response = await relayRpc(request("eth_getBalance", [ACCOUNT, "latest"]), async (input, options) => {
    assert.equal(options?.redirect, "manual");
    const body = payload(options); calls.push({ url: String(input), method: body.method });
    if (String(input) === PROVIDERS[8453][0]) return new Response(null, { status: 302, headers: { Location: "https://unapproved.example/redirect" } });
    return result(body, body.method === "eth_chainId" ? "0x2105" : "0x7");
  });
  assert.equal(response.status, 200);
  assert.equal((await response.json()).result, "0x7");
  assert.deepEqual(calls, [
    { url: PROVIDERS[8453][0], method: "eth_chainId" },
    { url: PROVIDERS[8453][1], method: "eth_chainId" },
    { url: PROVIDERS[8453][1], method: "eth_getBalance" },
  ]);
});

test("every redirect class rejects a submission after exactly one upstream attempt", async () => {
  for (const status of [300, 301, 302, 303, 304, 305, 307, 308, 399]) {
    const destinations: string[] = [];
    const response = await relayRpc(request("eth_sendRawTransaction", ["0x02abcd"]), async (input, options) => {
      destinations.push(String(input));
      assert.equal(options?.redirect, "manual");
      return new Response(null, { status, headers: { Location: "https://unapproved.example/redirect" } });
    });
    assert.equal(response.status, 502);
    assert.equal((await response.json()).error.code, -32099);
    assert.deepEqual(destinations, ["https://mainnet-sequencer.base.org"]);
  }
});

test("configured activity redirects cannot supply candidates or forward an API credential", async () => {
  const destinations: string[] = [];
  const response = await activityCandidates(new Request(`${ORIGIN}/api/activity?chainId=8453&address=${ACCOUNT}`), { BLOCKSCOUT_API_KEY: "test-fixture-key" }, async (input, options) => {
    destinations.push(String(input));
    assert.equal(options?.redirect, "manual");
    return new Response(JSON.stringify({ items: [{ transaction_hash: "0x" + "1".repeat(64) }] }), { status: 307, headers: { Location: "https://unapproved.example/redirect" } });
  });
  const data = await response.json();
  assert.equal(response.status, 200);
  assert.equal(data.partial, true);
  assert.deepEqual(data.hashes, []);
  assert.match(data.notice, /incomplete/);
  assert.equal(destinations.length, 2);
  assert.ok(destinations.every(value => new URL(value).hostname === "api.blockscout.com"));
});

test("diagnostics retain failure class without exposing account, transaction, credential or raw error", async () => {
  const original = console.warn;
  const records: Record<string, unknown>[] = [];
  console.warn = value => { records.push(JSON.parse(String(value))); };
  try {
    const raw = "0x02deadbeef";
    const sensitiveError = `private-error-marker ${ACCOUNT} ${raw} test-fixture-key`;
    const failing: typeof fetch = async () => { throw new TypeError(sensitiveError); };
    assert.equal((await relayRpc(request("eth_getBalance", [ACCOUNT, "latest"]), failing)).status, 503);
    assert.equal((await relayRpc(request("eth_getBalance", [ACCOUNT, "latest"]), failing)).status, 503);
    assert.equal((await relayRpc(request("eth_sendRawTransaction", [raw]), failing)).status, 502);
    await activityCandidates(new Request(`${ORIGIN}/api/activity?chainId=8453&address=${ACCOUNT}`), { BLOCKSCOUT_API_KEY: "test-fixture-key" }, failing);
    assert.equal(records.filter(value => value.reason === "type_error").length, 5);
    assert.equal(records.filter(value => value.event === "wallet_rpc_cooldown").length, 1);
    const allowed = new Set(["event", "chain_id", "method", "provider_host", "phase", "reason", "http_status", "rpc_code", "elapsed_ms", "remaining_ms"]);
    assert.ok(records.every(value => Object.keys(value).every(key => allowed.has(key))));
    const serialized = JSON.stringify(records);
    for (const secret of [ACCOUNT, raw, "private-error-marker", "test-fixture-key", "Authorization", "params"]) assert.equal(serialized.includes(secret), false);
  } finally { console.warn = original; }
});

test("diagnostic logger failure cannot change read failover, submission uncertainty or partial activity", async () => {
  const original = console.warn;
  console.warn = () => { throw new Error("logger unavailable"); };
  try {
    let reads = 0;
    const response = await relayRpc(request(), async (input, options) => {
      reads++;
      if (String(input) === PROVIDERS[8453][0]) throw new Error("first provider unavailable");
      return result(payload(options), "0x2105");
    });
    assert.equal(response.status, 200);
    assert.equal(reads, 2);
    let sends = 0;
    const submitted = await relayRpc(request("eth_sendRawTransaction", ["0x02abcd"]), async () => { sends++; throw new Error("submission response lost"); });
    assert.equal(submitted.status, 502);
    assert.equal((await submitted.json()).error.code, -32099);
    assert.equal(sends, 1);
    const activity = await activityCandidates(new Request(`${ORIGIN}/api/activity?chainId=8453&address=${ACCOUNT}`), { BLOCKSCOUT_API_KEY: "test-fixture-key" }, async () => { throw new Error("index unavailable"); });
    assert.equal(activity.status, 200);
    const data = await activity.json();
    assert.equal(data.partial, true);
    assert.deepEqual(data.hashes, []);
  } finally { console.warn = original; }
});
