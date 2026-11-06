import { test } from "node:test";
import assert from "node:assert/strict";
import { Journal } from "../src/journal";
import { WalletEngine } from "../src/engine";
import { blockHash, fixtureCore, freshJournal, FROM, prepare, receipt, sign, vault } from "./fixtures";
import type { BalanceSnapshot } from "../src/types";

test("a wallet cannot silently overwrite an existing encrypted vault", async () => {
  const { journal } = await freshJournal();
  assert.deepEqual(await journal.saveVault(vault), vault);
  await assert.rejects(journal.saveVault({ ...vault, salt: "11".repeat(16) }), /already exists/);
});
test("concurrent intents cannot reserve two payments on one account/network", async () => {
  const { journal } = await freshJournal();
  const results = await Promise.allSettled([journal.recordPrepared(prepare("first")), journal.recordPrepared(prepare("second", 1, 1))]);
  assert.equal(results.filter(v => v.status === "fulfilled").length, 1);
  assert.equal((await journal.list(1, FROM)).length, 1);
});
test("Ethereum and Base nonces are separate, and unchanged intent retries are idempotent", async () => {
  const { journal } = await freshJournal();
  const eth = prepare("eth"); const base = prepare("base", 8453);
  await journal.recordPrepared(eth); await journal.recordPrepared(base);
  await journal.recordPrepared(eth);
  assert.equal((await journal.events("eth")).length, 1);
  await assert.rejects(journal.recordPrepared({ ...eth, request: { ...eth.request, amount: "100" } }));
  assert.equal((await journal.list(1, FROM)).length, 1);
  assert.equal((await journal.list(8453, FROM)).length, 1);
});
test("unsigned cancellation releases a nonce; a signature never does", async () => {
  const { journal } = await freshJournal();
  await journal.recordPrepared(prepare("discard")); await journal.cancel("discard");
  const p = prepare("new-draft"); await journal.recordPrepared(p); await journal.recordSigned(sign(p));
  await assert.rejects(journal.cancel("new-draft"), /cannot be cancelled/);
  assert.equal((await journal.get("new-draft")).nonce_key, `1:${FROM.toLowerCase()}:0`);
});
test("signed bytes survive reopen and later uncertain retries do not downgrade acknowledgement", async () => {
  const { journal, factory, name } = await freshJournal(); const p = prepare(); const s = sign(p);
  await journal.recordPrepared(p); await journal.recordSigned(s); await journal.submission(s.intent_id, "pending"); await journal.submission(s.intent_id, "unknown");
  journal.close(); const reopened = await Journal.open(fixtureCore, factory, name);
  const row = await reopened.get(s.intent_id);
  assert.equal(row.phase, "pending"); assert.equal(row.signed?.raw_transaction, s.raw_transaction);
});
test("storage abort after a request success rolls back signature and prevents any broadcast", async () => {
  const { journal } = await freshJournal(); const p = prepare(); await journal.recordPrepared(p);
  const db: IDBDatabase = (journal as any).db;
  const original = db.transaction.bind(db);
  db.transaction = ((stores: string | string[], mode?: IDBTransactionMode, options?: IDBTransactionOptions) => {
    const tx = original(stores, mode, options);
    if (mode === "readwrite" && Array.from(typeof stores === "string" ? [stores] : stores).includes("payments")) {
      const originalStore = tx.objectStore.bind(tx);
      tx.objectStore = ((name: string) => {
        const store = originalStore(name);
        if (name === "payments") {
          const originalPut = store.put.bind(store);
          store.put = ((value: unknown, key?: IDBValidKey) => { const request = originalPut(value, key); request.addEventListener("success", () => tx.abort(), { once: true }); return request; }) as typeof store.put;
        }
        return store;
      }) as typeof tx.objectStore;
    }
    return tx;
  }) as typeof db.transaction;
  let broadcasts = 0;
  const rpc = { network: { chain_id: 1 }, revalidate: async () => {}, verifyChain: async () => {}, broadcast: async () => { broadcasts++; } };
  await assert.rejects(new WalletEngine(fixtureCore, journal, rpc as never).signAndSubmit(vault, "fixture-only", p));
  db.transaction = original;
  assert.equal(broadcasts, 0);
  assert.equal((await journal.get(p.request.intent_id)).signed, null);
});
test("receipt evidence is revalidated, finality is monotonic, actual Base fee is retained", async () => {
  const { journal } = await freshJournal(); const p = prepare("base-fee", 8453); const s = sign(p);
  await journal.recordPrepared(p); await journal.recordSigned(s);
  const r = receipt(s); r.l1Fee = "0x1388"; // Higher than quote, still authoritative outcome.
  const included = await journal.observe(s.intent_id, r, blockHash(), 99);
  assert.equal(included.phase, "included"); assert.equal(included.receipt?.actual_l1_fee_wei, "5000");
  assert.equal(included.receipt?.additional_fee_exceeded_estimate, true);
  await journal.observe(s.intent_id, r, blockHash(), 100);
  const row = await journal.observe(s.intent_id, r, blockHash(), 90);
  assert.equal(row.phase, "finalized"); assert.equal(row.receipt_proof?.finalized_height, 100);
  await assert.rejects(journal.observe(s.intent_id, { ...r, l1Fee: "0x1389" }, blockHash(), 100), /Conflicting receipt/);
  assert.equal((await journal.get(s.intent_id)).receipt?.actual_l1_fee_wei, "5000");
});
test("stored receipt projection corruption is detected on reload", async () => {
  const { journal } = await freshJournal(); const p = prepare(); const s = sign(p);
  await journal.recordPrepared(p); await journal.recordSigned(s); await journal.observe(s.intent_id, receipt(s), blockHash(), 100);
  const db: IDBDatabase = (journal as any).db;
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction("payments", "readwrite"); const store = tx.objectStore("payments"); const r = store.get(s.intent_id);
    r.onsuccess = () => { r.result.receipt.actual_fee_wei = "1"; store.put(r.result); };
    tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
  });
  await assert.rejects(journal.get(s.intent_id), /Stored receipt evidence/);
});
test("failed execution preserves charged gas; successful receipt missing expected transfer remains unresolved", async () => {
  const { journal } = await freshJournal(); const p = prepare(); const s = sign(p);
  await journal.recordPrepared(p); await journal.recordSigned(s);
  const r = receipt(s); r.logs = [];
  const unresolved = await journal.observe(s.intent_id, r, blockHash(), 100);
  assert.equal(unresolved.phase, "unresolved"); assert.equal(unresolved.execution, "unknown");
  const other = await freshJournal(); const f = prepare("failed", 8453); const fs = sign(f);
  await other.journal.recordPrepared(f); await other.journal.recordSigned(fs);
  const failed = await other.journal.observe(fs.intent_id, receipt(fs, 100, blockHash(), false), blockHash(), 100);
  assert.equal(failed.execution, "failed"); assert.notEqual(failed.receipt?.actual_fee_wei, "0");
});
test("reorgs append evidence and retain signed bytes; lost finality blocks sending", async () => {
  const { journal } = await freshJournal(); const p = prepare(); const s = sign(p);
  await journal.recordPrepared(p); await journal.recordSigned(s); await journal.observe(s.intent_id, receipt(s), blockHash(), 100);
  const count = (await journal.events(s.intent_id)).length;
  const row = await journal.reorg(s.intent_id, blockHash());
  assert.equal(row.phase, "unresolved"); assert.equal(row.inclusion, "unseen"); assert.equal(row.signed?.hash, s.hash);
  assert.equal((await journal.events(s.intent_id)).length, count + 1);
  await assert.rejects(journal.recordPrepared(prepare("next", 1, 1)), /Resolve the existing/);
});
test("older repeated balances cannot become latest and conflicts at one block are rejected", async () => {
  const { journal } = await freshJournal();
  const a: BalanceSnapshot = { chain_id: 1, address: FROM, block_number: 100, block_hash: blockHash(), balances: { ETH: "100", USDC: "200", EURC: "300" }, observed_at: 1 };
  const b = { ...a, block_number: 101, block_hash: blockHash("22"), observed_at: 2, balances: { ...a.balances, USDC: "250" } };
  await journal.recordBalance(a); await journal.recordBalance(b); await journal.recordBalance(a);
  assert.equal((await journal.latestBalance(1, FROM))?.block_number, 101);
  await assert.rejects(journal.recordBalance({ ...a, balances: { ...a.balances, USDC: "999" } }), /conflicting balances/);
  assert.equal(await journal.latestBalance(8453, FROM), null);
});
