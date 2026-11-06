// Public Hardhat fixture ONLY. These keys must never receive real funds.
import init, * as wasm from "../../build/wasm/wallet_core.js";
import { readFile } from "node:fs/promises";
import { IDBFactory } from "fake-indexeddb";
import { Journal } from "../src/journal";
import type { Asset, ChainId, CoreApi, PaymentRequest, PreparedPayment, SignedPayment, Vault } from "../src/types";

await init({ module_or_path: await readFile(new URL("../../build/wasm/wallet_core_bg.wasm", import.meta.url)) });
export { wasm };
export const PUBLIC_FIXTURE = "test test test test test test test test test test test junk";
export const FROM = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
export const TO = "0x1111111111111111111111111111111111111111";
export const KEY = new Uint8Array(32).fill(7);
const restored = JSON.parse(wasm.restore_wallet(PUBLIC_FIXTURE, KEY));
export const vault: Vault = { id: "primary", version: 1, kdf: "argon2id-64m-t3-p1", salt: "00".repeat(16), vault_hex: restored.vault_hex, address: restored.address, created_at: 1_700_000_000, backup_confirmed: true };
export const networks = JSON.parse(wasm.asset_registry()).networks;
export const fixtureCore: CoreApi = {
  registry: async () => JSON.parse(wasm.asset_registry()),
  create: async () => { throw new Error("Fixture create is not used."); },
  restore: async () => vault,
  authenticate: async () => FROM,
  reveal: async () => PUBLIC_FIXTURE,
  prepare: async request => JSON.parse(wasm.prepare_payment(JSON.stringify(request))),
  sign: async (_vault, _password, prepared) => sign(prepared),
  validatePrepared: async prepared => JSON.parse(wasm.validate_prepared_json(JSON.stringify(prepared))),
  validateSigned: async (prepared, signed) => JSON.parse(wasm.validate_signed_for_prepared(JSON.stringify(prepared), JSON.stringify(signed))),
  validateReceipt: async (signed, receipt, canonicalHash, finalized) => JSON.parse(wasm.validate_receipt(JSON.stringify(signed), JSON.stringify(receipt), canonicalHash, BigInt(finalized))),
  validateActivity: async (chain, account, proof) => JSON.parse(wasm.validate_activity(BigInt(chain), account, JSON.stringify(proof.transaction), JSON.stringify(proof.receipt), JSON.stringify(proof.canonical_block), BigInt(proof.finalized_height))),
};
export function request(id = "fixture-1", chain: ChainId = 1, nonce = 0, asset: Asset = "USDC"): PaymentRequest {
  return { intent_id: id, chain_id: chain, from: FROM, to: TO, asset, amount: asset === "ETH" ? "0.001" : "1.25", nonce,
    gas_limit: asset === "ETH" ? 21_000 : 60_000, max_fee_per_gas: "2000000000", max_priority_fee_per_gas: "1000000000",
    l1_fee_estimate_wei: chain === 8453 ? "1000" : "0", operator_fee_estimate_wei: chain === 8453 ? "100" : "0" };
}
export function prepare(id = "fixture-1", chain: ChainId = 1, nonce = 0, asset: Asset = "USDC"): PreparedPayment {
  return JSON.parse(wasm.prepare_payment(JSON.stringify(request(id, chain, nonce, asset))));
}
export function sign(prepared: PreparedPayment): SignedPayment {
  return JSON.parse(wasm.sign_payment(vault.vault_hex, KEY, JSON.stringify(prepared), prepared.approval_digest));
}
export const blockHash = (byte = "11") => `0x${byte.repeat(32)}`;
export function receipt(signed: SignedPayment, block = 100, block_hash = blockHash(), success = true): Record<string, unknown> {
  const inspected = JSON.parse(wasm.inspect_signed(signed.raw_transaction));
  const token = signed.asset !== "ETH";
  return {
    transactionHash: signed.hash, blockHash: block_hash, blockNumber: `0x${block.toString(16)}`, from: signed.from, to: inspected.transaction_to,
    status: success ? "0x1" : "0x0", gasUsed: token ? "0xc350" : "0x5208", effectiveGasPrice: "0x59682f00", type: "0x2", contractAddress: null,
    ...(signed.chain_id === 8453 ? { l1Fee: "0x3e8", operatorFee: "0x64" } : {}),
    logs: token && success ? [{ address: inspected.transaction_to, topics: ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef", `0x${FROM.slice(2).toLowerCase().padStart(64, "0")}`, `0x${TO.slice(2).padStart(64, "0")}`], data: `0x${BigInt(signed.amount_units).toString(16).padStart(64, "0")}`, transactionHash: signed.hash, blockHash: block_hash, blockNumber: `0x${block.toString(16)}`, logIndex: "0x0", removed: false }] : [],
  };
}
export async function freshJournal() {
  const factory = new IDBFactory();
  const name = crypto.randomUUID();
  const journal = await Journal.open(fixtureCore, factory, name);
  await journal.saveVault(vault);
  return { journal, factory, name };
}
