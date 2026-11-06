/** Read-only mainnet acceptance check. Never imports a private key, signs,
 * creates a wallet, sends raw transactions or moves funds. */
import assert from "node:assert/strict";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import init, * as wasm from "../build/wasm/wallet_core.js";
import { NetworkClient } from "../web/src/network";
import { relayRpc } from "../worker/relay";
import type { Network } from "../web/src/types";

await init({ module_or_path: await readFile(new URL("../build/wasm/wallet_core_bg.wasm", import.meta.url)) });
const networks: Network[] = JSON.parse(wasm.asset_registry()).networks;
const ACCOUNT = "0x000000000000000000000000000000000000dEaD";
const RECIPIENT = "0x1111111111111111111111111111111111111111";
let reads = 0;
const readOnlyFetch: typeof fetch = async (input, options) => {
  if (options?.body) {
    const body = JSON.parse(String(options.body));
    assert.notEqual(body.method, "eth_sendRawTransaction", "Live verification cannot submit a transaction.");
    assert.ok(String(body.method).startsWith("eth_"));
  }
  reads++;
  const begin = Date.now();
  try {
    const response = await fetch(input, options);
    console.log(JSON.stringify({ rpc: JSON.parse(String(options?.body)).method, provider: new URL(String(input)).hostname, status: response.status, milliseconds: Date.now() - begin }));
    return response;
  } catch (failure) {
    console.log(JSON.stringify({ rpc: JSON.parse(String(options?.body)).method, provider: new URL(String(input)).hostname, error: String(failure), milliseconds: Date.now() - begin }));
    throw failure;
  }
};
const hostedFetch: typeof fetch = async (input, options) => {
  assert.equal(String(input), "/api/rpc");
  return relayRpc(new Request("https://wallet.example/api/rpc", { ...options, headers: { "Content-Type": "application/json", Origin: "https://wallet.example" } }), readOnlyFetch);
};
const report: Record<string, unknown> = { time: new Date().toISOString(), mode: "read-only live mainnet; in-process production relay; no funded transaction", checks: [] };
const checks = report.checks as Record<string, unknown>[];
for (const network of networks) {
  const client = new NetworkClient(network, hostedFetch, { relay: "/api/rpc", timeoutMs: 52_000 });
  const check: Record<string, unknown> = { chain_id: network.chain_id };
  try {
    const snapshot = await client.balances(ACCOUNT);
    assert.equal(snapshot.address, ACCOUNT);
    assert.equal(snapshot.chain_id, network.chain_id);
    assert.ok(snapshot.block_number > 0);
    check.balances = { result: "passed", account: ACCOUNT, block: snapshot.block_number, values: snapshot.balances, provider: client.connectionInfo().provider };
  } catch (failure) { check.balances = { result: "failed", reason: String(failure) }; }
  try {
    const fee = await client.preview({ from: ACCOUNT, to: RECIPIENT, asset: "USDC", amount: "0.01" });
    assert.equal(fee.chain_id, network.chain_id);
    assert.ok(BigInt(fee.estimated_total_fee_wei) > 0n);
    assert.ok(BigInt(fee.fee_reserve_wei) >= BigInt(fee.estimated_total_fee_wei));
    check.fee_preview = { result: "passed", estimated_total_fee_wei: fee.estimated_total_fee_wei, fee_reserve_wei: fee.fee_reserve_wei, estimate_source: fee.estimate_source, shortages: fee.shortages, warnings: fee.warnings };
  } catch (failure) { check.fee_preview = { result: "failed", reason: String(failure) }; }
  checks.push(check);
  console.log(JSON.stringify(check));
}
report.rpc_requests = reads;
await mkdir(new URL("../docs/evidence/", import.meta.url), { recursive: true });
await writeFile(new URL("../docs/evidence/mainnet-live-reads-v03.json", import.meta.url), JSON.stringify(report, null, 2) + "\n");
if (checks.some(check => [check.balances, check.fee_preview].some(result => (result as Record<string, unknown>).result !== "passed"))) process.exitCode = 1;
