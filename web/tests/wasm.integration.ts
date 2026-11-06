import { test } from "node:test";
import assert from "node:assert/strict";
import { wasm, PUBLIC_FIXTURE, FROM, networks, prepare, sign, receipt, blockHash } from "./fixtures";

test("actual release WASM restores the standard fixture and authenticates password-derived vaults", () => {
  const salt = new Uint8Array(16).fill(42);
  const key = wasm.derive_vault_key("public test password only", salt);
  const again = wasm.derive_vault_key("public test password only", salt);
  assert.deepEqual(key, again); assert.equal(key.length, 32);
  const restored = JSON.parse(wasm.restore_wallet(PUBLIC_FIXTURE, key));
  assert.equal(wasm.wallet_address(restored.vault_hex, key), FROM);
  assert.equal(wasm.reveal_mnemonic(restored.vault_hex, key), PUBLIC_FIXTURE);
  const wrong = wasm.derive_vault_key("wrong public test password", salt);
  assert.throws(() => wasm.wallet_address(restored.vault_hex, wrong));
  key.fill(0); again.fill(0); wrong.fill(0);
});
test("actual WASM generates 24 words and cannot sign a tampered review", () => {
  const key = crypto.getRandomValues(new Uint8Array(32));
  const generated = JSON.parse(wasm.generate_wallet(key));
  assert.equal(generated.mnemonic.split(" ").length, 24);
  assert.equal(wasm.wallet_address(generated.vault_hex, key), generated.address);
  const p = prepare("tamper");
  assert.throws(() => wasm.validate_prepared_json(JSON.stringify({ ...p, request: { ...p.request, to: "0x2222222222222222222222222222222222222222" } })));
  key.fill(0); generated.mnemonic = "";
});
test("actual release WASM authorizes native USDC on both real mainnets and validates its receipts", () => {
  assert.deepEqual(networks.map((n: any) => n.chain_id).sort((a: number, b: number) => a - b), [1, 8453]);
  assert.equal(networks.find((n: any) => n.chain_id === 8453).assets.find((a: any) => a.symbol === "USDC").contract, "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
  for (const chain of [1, 8453] as const) {
    const p = prepare(`wasm-${chain}`, chain); const s = sign(p);
    assert.equal(JSON.parse(wasm.inspect_signed(s.raw_transaction)).chain_id, chain);
    assert.equal(JSON.parse(wasm.validate_signed_for_prepared(JSON.stringify(p), JSON.stringify(s))).hash, s.hash);
    const result = JSON.parse(wasm.validate_receipt(JSON.stringify(s), JSON.stringify(receipt(s)), blockHash(), 100n));
    assert.equal(result.phase, "finalized"); assert.equal(result.execution, "success");
    assert.equal(result.actual_l1_fee_wei, chain === 8453 ? "1000" : "0");
  }
  const testnet = prepare("bad-network"); testnet.request.chain_id = 11155111 as never;
  assert.throws(() => wasm.prepare_payment(JSON.stringify(testnet.request)));
});
