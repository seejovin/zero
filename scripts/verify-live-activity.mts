/** Read-only acceptance: discover a public USDC log and independently verify
 * its transaction, receipt and canonical block through the shipped validator.
 * No wallet, private key, signature or broadcast is created by this script. */
import assert from "node:assert/strict";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import init, * as wasm from "../build/wasm/wallet_core.js";
import { NetworkClient } from "../web/src/network";
import { relayRpc } from "../worker/relay";
import type { Network } from "../web/src/types";

await init({ module_or_path: await readFile(new URL("../build/wasm/wallet_core_bg.wasm", import.meta.url)) });
const networks: Network[] = JSON.parse(wasm.asset_registry()).networks;
const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const checks: Record<string, unknown>[] = [];
const report = { time: new Date().toISOString(), mode: "read-only public mainnet fixtures; actual release WASM and in-process production relay; no user wallet or funded send", checks };
let sequence = 0;
const readOnlyFetch: typeof fetch = async (input, options) => {
  const body = JSON.parse(String(options?.body));
  assert.notEqual(body.method, "eth_sendRawTransaction");
  assert.ok(String(body.method).startsWith("eth_"));
  return fetch(input, options);
};
const hostedFetch: typeof fetch = async (input, options) => {
  assert.equal(String(input), "/api/rpc");
  return relayRpc(new Request("https://wallet.example/api/rpc", { ...options, headers: { "Content-Type": "application/json", Origin: "https://wallet.example" } }), readOnlyFetch);
};
for (const network of networks) {
  try {
    const client = new NetworkClient(network, hostedFetch, { relay: "/api/rpc", timeoutMs: 52_000 });
    const tip = await client.block("latest", true);
    const height = BigInt(tip.number);
    const contract = network.assets.find(asset => asset.symbol === "USDC")!.contract!;
    // Discovery only: this public fixture query has no wallet participant.
    // Production relay intentionally forbids unfiltered participant scans.
    const provider = network.chain_id === 1 ? "https://ethereum-rpc.publicnode.com" : "https://base-rpc.publicnode.com";
    const response = await readOnlyFetch(provider, {
      method: "POST", headers: { "Content-Type": "application/json" }, signal: AbortSignal.timeout(20_000),
      body: JSON.stringify({ jsonrpc: "2.0", id: ++sequence, method: "eth_getLogs", params: [{ address: contract, fromBlock: `0x${(height - 3n).toString(16)}`, toBlock: tip.number, topics: [TRANSFER] }] }),
    });
    assert.equal(response.status, 200);
    const discovered = await response.json() as { result?: Array<Record<string, any>> };
    assert.ok(Array.isArray(discovered.result));
    const log = discovered.result.find(item => item.topics?.length === 3 && BigInt(item.data) > 0n && BigInt(item.topics[2]) > 0n && item.topics[1] !== item.topics[2]);
    assert.ok(log, "A recent public USDC transfer fixture is required.");
    const recipient = `0x${String(log.topics[2]).slice(-40)}`;
    const filtered = await client.read("eth_getLogs", [{ address: network.assets.filter(asset => asset.contract).map(asset => asset.contract), fromBlock: log.blockNumber, toBlock: log.blockNumber, topics: [TRANSFER, null, log.topics[2]] }]);
    assert.ok(Array.isArray(filtered) && filtered.some(item => item.transactionHash === log.transactionHash && item.logIndex === log.logIndex), "The actual wallet's participant-filtered discovery request must find this transfer through the relay.");
    const [transaction, receipt, block, finalized] = await Promise.all([
      client.read("eth_getTransactionByHash", [log.transactionHash]),
      client.read("eth_getTransactionReceipt", [log.transactionHash]),
      client.block(log.blockNumber),
      client.block("finalized"),
    ]);
    const validated = JSON.parse(wasm.validate_activity(BigInt(network.chain_id), recipient, JSON.stringify(transaction), JSON.stringify(receipt), JSON.stringify(block), BigInt(finalized.number)));
    await client.assertCanonical(block);
    const expected = validated.activities.find((item: Record<string, any>) => item.transaction_hash === String(log.transactionHash).toLowerCase() && item.log_index === Number(BigInt(log.logIndex)));
    assert.ok(expected, "The live transfer must be returned by the shared validator.");
    assert.equal(expected.asset, "USDC");
    assert.equal(expected.direction, "received");
    assert.equal(expected.to.toLowerCase(), recipient.toLowerCase());
    assert.equal(expected.asset_contract.toLowerCase(), contract.toLowerCase());
    assert.equal(expected.amount_units, BigInt(log.data).toString());
    const check = { chain_id: network.chain_id, result: "passed", participant_filtered_discovery: "passed", transaction_hash: log.transactionHash, block_number: Number(BigInt(log.blockNumber)), asset: expected.asset, direction: expected.direction, amount_units: expected.amount_units, recipient, inclusion: expected.inclusion };
    checks.push(check); console.log(JSON.stringify(check));
  } catch (failure) {
    const check = { chain_id: network.chain_id, result: "failed", reason: String(failure) };
    checks.push(check); console.log(JSON.stringify(check));
  }
}
await mkdir(new URL("../docs/evidence/", import.meta.url), { recursive: true });
await writeFile(new URL("../docs/evidence/mainnet-live-activity-v03.json", import.meta.url), JSON.stringify(report, null, 2) + "\n");
if (checks.some(check => check.result !== "passed")) process.exitCode = 1;
