import { test } from "node:test";
import assert from "node:assert/strict";
import { WalletEngine } from "../src/engine";
import { blockHash, fixtureCore, freshJournal, prepare, receipt, sign, vault } from "./fixtures";

test("an uncertain broadcast preserves exactly one signed payment and never invents failure", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); await journal.recordPrepared(prepared);
  let calls = 0;
  const rpc = { network: { chain_id: 1 }, revalidate: async () => {}, verifyChain: async () => {}, broadcast: async () => { calls++; throw new Error("Simulated timeout after network acceptance."); } };
  const result = await new WalletEngine(fixtureCore, journal, rpc as never).signAndSubmit(vault, "public-fixture", prepared);
  assert.equal(calls, 1); assert.equal(result.acknowledged, false); assert.equal(result.record.phase, "unknown");
  assert.equal(result.record.signed?.hash, sign(prepared).hash);
  assert.equal((await journal.list(1, vault.address)).length, 1);
});
test("locking during the last chain check prevents broadcast of the saved signature", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); await journal.recordPrepared(prepared);
  let authorized = true; let calls = 0;
  const rpc = { network: { chain_id: 1 }, revalidate: async () => {}, verifyChain: async () => { authorized = false; }, broadcast: async () => { calls++; } };
  await assert.rejects(new WalletEngine(fixtureCore, journal, rpc as never).signAndSubmit(vault, "public-fixture", prepared, () => authorized), /Wallet locked/);
  assert.equal(calls, 0); assert.equal((await journal.get(prepared.request.intent_id)).phase, "signed");
});
test("an async protection check prevents signing after revocation during revalidation", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); await journal.recordPrepared(prepared);
  let authorized = true; let signatures = 0; let broadcasts = 0;
  const core = { ...fixtureCore, sign: async () => { signatures++; return sign(prepared); } };
  const rpc = { network: { chain_id: 1 }, revalidate: async () => { authorized = false; }, verifyChain: async () => {}, broadcast: async () => { broadcasts++; } };
  const check = async () => { await Promise.resolve(); return authorized; };
  await assert.rejects(new WalletEngine(core, journal, rpc as never).signAndSubmit(vault, "public-fixture", prepared, check), /Wallet locked before signing/);
  assert.equal(signatures, 0); assert.equal(broadcasts, 0);
  const saved = await journal.get(prepared.request.intent_id);
  assert.equal(saved.phase, "prepared"); assert.equal(saved.signed, null);
});
test("an async protection check discards a signature completed after lock revocation", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); await journal.recordPrepared(prepared);
  let authorized = true; let signatures = 0; let broadcasts = 0;
  const core = { ...fixtureCore, sign: async () => { signatures++; const signed = sign(prepared); authorized = false; return signed; } };
  const rpc = { network: { chain_id: 1 }, revalidate: async () => {}, verifyChain: async () => {}, broadcast: async () => { broadcasts++; } };
  const check = async () => { await Promise.resolve(); return authorized; };
  await assert.rejects(new WalletEngine(core, journal, rpc as never).signAndSubmit(vault, "public-fixture", prepared, check), /Wallet locked/);
  assert.equal(signatures, 1); assert.equal(broadcasts, 0);
  const saved = await journal.get(prepared.request.intent_id);
  assert.equal(saved.phase, "prepared"); assert.equal(saved.signed, null);
});
test("an async protection check stops broadcast after the last chain check revokes access", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); await journal.recordPrepared(prepared);
  let authorized = true; let broadcasts = 0;
  const rpc = { network: { chain_id: 1 }, revalidate: async () => {}, verifyChain: async () => { authorized = false; }, broadcast: async () => { broadcasts++; } };
  const check = async () => { await Promise.resolve(); return authorized; };
  await assert.rejects(new WalletEngine(fixtureCore, journal, rpc as never).signAndSubmit(vault, "public-fixture", prepared, check), /Wallet locked/);
  assert.equal(broadcasts, 0);
  const saved = await journal.get(prepared.request.intent_id);
  assert.equal(saved.phase, "signed"); assert.equal(saved.signed?.hash, sign(prepared).hash);
});
test("retry awaits a revoked async protection check and preserves the original signed bytes", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); const signed = sign(prepared);
  await journal.recordPrepared(prepared); await journal.recordSigned(signed); await journal.submission(signed.intent_id, "unknown");
  let authorized = true; let chainChecks = 0; let broadcasts = 0;
  const core = { ...fixtureCore, sign: async () => { throw new Error("Retry must not sign."); } };
  const rpc = {
    network: { chain_id: 1 }, verifyChain: async () => { chainChecks++; if (chainChecks === 2) authorized = false; },
    receipt: async () => null, transactionPresent: async () => false, broadcast: async () => { broadcasts++; },
  };
  const check = async () => { await Promise.resolve(); return authorized; };
  await assert.rejects(new WalletEngine(core, journal, rpc as never).retry(signed.intent_id, check), /Wallet locked/);
  assert.equal(chainChecks, 2); assert.equal(broadcasts, 0);
  const saved = await journal.get(signed.intent_id);
  assert.equal(saved.phase, "unknown"); assert.equal(saved.signed?.raw_transaction, signed.raw_transaction);
});
test("retry submits identical saved bytes without signing again", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); const signed = sign(prepared);
  await journal.recordPrepared(prepared); await journal.recordSigned(signed); await journal.submission(signed.intent_id, "unknown");
  let raw = "";
  const core = { ...fixtureCore, sign: async () => { throw new Error("Retry must not sign."); } };
  const rpc = { network: { chain_id: 1 }, verifyChain: async () => {}, receipt: async () => null, transactionPresent: async () => false, broadcast: async (s: any) => { raw = s.raw_transaction; } };
  const result = await new WalletEngine(core, journal, rpc as never).retry(signed.intent_id);
  assert.equal(raw, signed.raw_transaction); assert.equal(result.record.phase, "pending");
});
test("a canonical receipt that changes after finalized lookup is not accepted", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); const signed = sign(prepared);
  await journal.recordPrepared(prepared); await journal.recordSigned(signed);
  let finalityWasRead = false;
  const rpc = {
    network: { chain_id: 1 }, verifyChain: async () => {}, receipt: async () => receipt(signed), receiptFees: async (r: any) => r,
    block: async (tag: string) => { if (tag === "finalized") { finalityWasRead = true; return { number: "0x64", hash: blockHash("ff") }; } return { number: "0x64", hash: blockHash() }; },
    assertCanonical: async () => { assert.equal(finalityWasRead, true); throw new Error("Simulated reorganization race."); },
  };
  await assert.rejects(new WalletEngine(fixtureCore, journal, rpc as never).reconcile(signed.intent_id), /reorganization race/);
  assert.equal((await journal.get(signed.intent_id)).receipt, null);
});
test("conflicting new receipt cannot erase a still-canonical earlier receipt", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); const signed = sign(prepared);
  await journal.recordPrepared(prepared); await journal.recordSigned(signed); await journal.observe(signed.intent_id, receipt(signed), blockHash(), 99);
  const rpc = { network: { chain_id: 1 }, verifyChain: async () => {}, receipt: async () => receipt(signed, 101, blockHash("22")), block: async (tag: string) => { assert.equal(tag, "0x64"); return { number: tag, hash: blockHash() }; } };
  await assert.rejects(new WalletEngine(fixtureCore, journal, rpc as never).reconcile(signed.intent_id), /earlier block remains canonical/);
  assert.equal((await journal.get(signed.intent_id)).receipt?.block_hash, blockHash());
});
test("a temporarily missing receipt does not erase canonical inclusion", async () => {
  const { journal } = await freshJournal(); const prepared = prepare(); const signed = sign(prepared);
  await journal.recordPrepared(prepared); await journal.recordSigned(signed); await journal.observe(signed.intent_id, receipt(signed), blockHash(), 99);
  const rpc = { network: { chain_id: 1 }, verifyChain: async () => {}, receipt: async () => null, block: async () => ({ number: "0x64", hash: blockHash() }) };
  await assert.rejects(new WalletEngine(fixtureCore, journal, rpc as never).reconcile(signed.intent_id), /temporarily omitted/);
  assert.equal((await journal.get(signed.intent_id)).phase, "included");
});
