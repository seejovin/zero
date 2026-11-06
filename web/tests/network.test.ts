import { test } from "node:test";
import assert from "node:assert/strict";
import { NetworkClient, rpcTransaction, minimumGas } from "../src/network";
import { blockHash, FROM, TO, networks, prepare, sign, receipt } from "./fixtures";
import { hexQuantity } from "../src/units";

const base = networks.find((n: any) => n.chain_id === 8453);
const ethereum = networks.find((n: any) => n.chain_id === 1);
const word = (value: bigint | number) => `0x${BigInt(value).toString(16).padStart(64, "0")}`;
function fetchRpc(handler: (method: string, params: any[], endpoint: string) => unknown | Promise<unknown>): typeof fetch {
  return (async (endpoint: string | URL | Request, init?: RequestInit) => {
    const body = JSON.parse(init!.body as string);
    const result = await handler(body.method, body.params, String(endpoint));
    return new Response(JSON.stringify({ jsonrpc: "2.0", id: body.id, result }), { headers: { "Content-Type": "application/json" } });
  }) as typeof fetch;
}
test("RPC rejects a wrong chain, wrong response id, duplicate receipt fields, and oversized body", async () => {
  await assert.rejects(new NetworkClient(ethereum, fetchRpc(() => "0x2105")).verifyChain(), /wrong network/);
  const wrongId = (async () => new Response('{"jsonrpc":"2.0","id":999,"result":"0x1"}')) as typeof fetch;
  await assert.rejects(new NetworkClient(ethereum, wrongId).verifyChain(), /mismatched/);
  const duplicate = (async (_url: unknown, init: RequestInit) => {
    const id = JSON.parse(init.body as string).id;
    return new Response(`{"jsonrpc":"2.0","id":${id},"result":{"status":"0x0","status":"0x1"}}`);
  }) as typeof fetch;
  await assert.rejects(new NetworkClient(ethereum, duplicate).read("eth_getTransactionReceipt", [blockHash()]));
  const huge = (async () => new Response("{}", { headers: { "Content-Length": "1048577" } })) as typeof fetch;
  await assert.rejects(new NetworkClient(ethereum, huge).verifyChain(), /size limit/);
});
test("RPC rejects an oversized streaming response even without a content length", async () => {
  const fetcher = (async () => new Response(" ".repeat(1_048_577))) as typeof fetch;
  await assert.rejects(new NetworkClient(ethereum, fetcher).verifyChain(), /size limit/);
});
test("only approved endpoints and methods are available", async () => {
  assert.throws(() => new NetworkClient({ ...ethereum, rpc_url: "https://evil.invalid" }));
  assert.throws(() => new NetworkClient({ ...ethereum, chain_id: 11155111 }));
  await assert.rejects(new NetworkClient(ethereum, fetchRpc(() => null)).read("eth_sendRawTransaction", ["0x02"]));
});
test("Base quote includes upper-bound L1/operator estimates and exact calldata", async () => {
  const calls: { method: string; params: any[] }[] = [];
  const latest = { number: "0x64", hash: blockHash(), baseFeePerGas: "0x3b9aca00", timestamp: hexQuantity(Math.floor(Date.now() / 1000)) };
  const provider = fetchRpc((method, params) => {
    calls.push({ method, params });
    switch (method) {
      case "eth_chainId": return "0x2105";
      case "eth_getBlockByNumber": return latest;
      case "eth_getTransactionCount": return "0x0";
      case "eth_maxPriorityFeePerGas": return "0x5f5e100";
      case "eth_getBalance": return "0x1bc16d674ec80000";
      case "eth_getCode": return params[0].toLowerCase() === FROM.toLowerCase() ? "0x" : "0x60006000";
      case "eth_estimateGas": return "0xc350";
      case "eth_call": {
        const selector = params[0].data.slice(0, 10);
        if (selector === "0x70a08231") return word(20_000_000);
        if (selector === "0xa9059cbb") return word(1);
        if (selector === "0x313ce567") return word(6);
        if (selector === "0xf1c7a58b") return word(1000);
        if (selector === "0x275aedd2") return word(100);
        throw new Error("Unexpected contract call.");
      }
      default: throw new Error("Unexpected RPC method.");
    }
  });
  const quoted = await new NetworkClient(base, provider).quote("quote-fixture", FROM, TO, "USDC", "1.25");
  assert.equal(quoted.request.gas_limit, 62_500);
  assert.equal(quoted.request.max_fee_per_gas, "2100000000");
  assert.equal(quoted.request.l1_fee_estimate_wei, "1000"); assert.equal(quoted.request.operator_fee_estimate_wei, "100");
  const upper = calls.find(c => c.method === "eth_call" && c.params[0].data.startsWith("0xf1c7a58b"))!;
  assert.equal(BigInt(`0x${upper.params[0].data.slice(10)}`), 1024n);
  assert.deepEqual(upper.params[1], { blockHash: blockHash(), requireCanonical: true });
  const call = rpcTransaction(base, FROM, TO, "USDC", 1_250_000n);
  assert.equal(call.to, base.assets.find((a: any) => a.symbol === "USDC").contract);
  assert.equal(call.value, "0x0"); assert.equal(call.data.length, 138);
  assert.ok(minimumGas(call.data) > 21_000n);
});
test("Base actual operator fee is queried at the receipt block and mismatches/missing L1 fee fail", async () => {
  const signed = sign(prepare("fee", 8453));
  const raw = receipt(signed); delete raw.operatorFee;
  let observed: any;
  const client = new NetworkClient(base, fetchRpc((method, params) => { if(method==="eth_chainId") return "0x2105"; assert.equal(method, "eth_call"); observed = params; return word(123); }));
  const result = await client.receiptFees(raw);
  assert.equal(result.operatorFee, "0x7b");
  assert.deepEqual(observed[1], { blockHash: blockHash(), requireCanonical: true });
  assert.equal(BigInt(`0x${observed[0].data.slice(10)}`), 50_000n);
  await assert.rejects(client.receiptFees({ ...raw, operatorFee: "0x0" }), /disagrees/);
  const missingL1 = { ...raw }; delete missingL1.l1Fee;
  await assert.rejects(client.receiptFees(missingL1));
});
test("Base submission uses the sequencer and validates the returned transaction hash", async () => {
  const signed = sign(prepare("broadcast-fixture", 8453));
  let endpoint = ""; let method = ""; let raw = "";
  const client = new NetworkClient(base, fetchRpc((m, params, url) => { endpoint = url; method = m; raw = params[0]; return signed.hash; }));
  await client.broadcast(signed);
  assert.equal(endpoint, "https://mainnet-sequencer.base.org"); assert.equal(method, "eth_sendRawTransaction"); assert.equal(raw, signed.raw_transaction);
  await assert.rejects(new NetworkClient(base, fetchRpc(() => blockHash("ff"))).broadcast(signed), /different transaction hash/);
});
