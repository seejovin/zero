// Independent differential test. The fixture phrase is public and unsafe for funds.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { HDNodeWallet, Interface, Transaction, keccak256, parseUnits } from "ethers";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const cargo = process.env.WALLET_CARGO || "cargo";
const fixtures = JSON.parse(execFileSync(cargo, [
  "run", "--quiet", "--locked", "-p", "wallet_core", "--example", "differential_vectors"
], { cwd: root, encoding: "utf8", maxBuffer: 1024 * 1024 }));
const fixtureWallet = HDNodeWallet.fromPhrase(
  "test test test test test test test test test test test junk", "", "m/44'/60'/0'/0/0"
);
assert.equal(fixtureWallet.address, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
const tokens = {
  1: {
    USDC: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    EURC: "0x1aBaEA1f7C830bD89Acc67eC4af516284b1bC33c",
    USDT: "0xdAC17F958D2ee523a2206206994597C13D831ec7",
  },
  8453: {
    USDC: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
    EURC: "0x60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42",
  },
  10: {
    USDC: "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85",
  },
  42161: {
    USDC: "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
  },
};
tokens[43114] = {
  USDC: "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E",
  EURC: "0xC891EB4cbdEFf6e073e859e987815Ed1505c2ACD",
  USDT: "0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7",
};
tokens[137] = { USDC: "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359" };
tokens[56] = {};
const native = { 1:"ETH", 8453:"ETH", 10:"ETH", 42161:"ETH", 43114:"AVAX", 137:"POL", 56:"BNB" };
const erc20 = new Interface(["function transfer(address to, uint256 value) returns (bool)"]);
for (const { request: r, prepared: p, signed: s } of fixtures) {
  assert.ok(tokens[r.chain_id], `Unsupported fixture chain ${r.chain_id}`);
  assert.ok(r.asset === native[r.chain_id] || tokens[r.chain_id][r.asset], "Unsupported token must not become a native transfer");
  const units = parseUnits(r.amount, r.asset === native[r.chain_id] ? 18 : 6);
  const tx = {
    type: 2,
    chainId: r.chain_id,
    nonce: r.nonce,
    maxPriorityFeePerGas: BigInt(r.max_priority_fee_per_gas),
    maxFeePerGas: BigInt(r.max_fee_per_gas),
    gasLimit: BigInt(r.gas_limit),
    to: r.asset === native[r.chain_id] ? r.to : tokens[r.chain_id][r.asset],
    value: r.asset === native[r.chain_id] ? units : 0n,
    data: r.asset === native[r.chain_id] ? "0x" : erc20.encodeFunctionData("transfer", [r.to, units]),
    accessList: [],
  };
  const independent = Transaction.from(tx);
  assert.equal(p.unsigned_transaction.toLowerCase(), independent.unsignedSerialized.toLowerCase());
  assert.equal(p.amount_units, units.toString());
  assert.equal(p.max_fee_wei, (tx.gasLimit * tx.maxFeePerGas).toString());
  const extra = BigInt(r.l1_fee_estimate_wei) + BigInt(r.operator_fee_estimate_wei);
  assert.equal(p.estimated_total_fee_wei, (tx.gasLimit * tx.maxFeePerGas + extra).toString());
  assert.equal(p.fee_reserve_wei, (tx.gasLimit * tx.maxFeePerGas + 2n * extra).toString());
  const signedByEthers = await fixtureWallet.signTransaction(tx);
  assert.equal(s.raw_transaction.toLowerCase(), signedByEthers.toLowerCase());
  const decoded = Transaction.from(s.raw_transaction);
  assert.equal(decoded.from, fixtureWallet.address);
  assert.equal(decoded.hash, s.hash.toLowerCase());
  assert.equal(keccak256(s.raw_transaction), s.hash.toLowerCase());
  assert.equal(decoded.chainId, BigInt(r.chain_id));
  assert.equal(decoded.unsignedSerialized.toLowerCase(), p.unsigned_transaction.toLowerCase());
}
assert.deepEqual([...new Set(fixtures.map(f => f.request.chain_id))].sort((a, b) => a-b), [1,10,56,137,8453,42161,43114]);
for (const chainId of [1,10,56,137,8453,42161,43114]) {
  const expectedAssets = [native[chainId], ...Object.keys(tokens[chainId])].sort();
  assert.deepEqual([...new Set(fixtures.filter(f => f.request.chain_id === chainId).map(f => f.request.asset))].sort(), expectedAssets);
}
console.log(`PASS: ${fixtures.length} Rust transactions independently matched ethers: derivation, amount units, fees, unsigned encoding, deterministic signature, recovered sender and hash.`);

