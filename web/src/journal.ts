import type { ActivityProof, ChainActivity, BalanceSnapshot, ChainId, CoreApi, JournalEvent, PaymentRecord, PreparedPayment, SignedPayment, Vault, VaultProtectionRecord, VaultProtectionState, VaultProtectionAttempt } from "./types";
import { accountKey, constantJson, decimal, hash, sameAddress, validateVault } from "./units";
import { assertSameVault, sameProtectionPolicy, validateProtection } from "./protection-format";

const DB_NAME = "local-wallet-mainnet-v02";
const now = () => Math.floor(Date.now() / 1000);
const conflict = () => new Error("The journal changed in another operation. Refresh before continuing.");
export const isActive = (record: PaymentRecord): boolean => record.phase !== "cancelled" && record.phase !== "finalized";
function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => { req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error ?? new Error("Local storage request failed.")); });
}
function complete(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(new Error("Local storage did not commit. No new transaction may be broadcast."));
    tx.onerror = () => { /* onabort is authoritative; a request success is not a commit. */ };
  });
}
function assertRecord(record: PaymentRecord, chain: ChainId, from: string): void {
  const p = record.prepared;
  if (!record || record.intent_id !== p?.request?.intent_id || record.account !== accountKey(chain, from) || p.request.chain_id !== chain || !sameAddress(p.request.from, from) || !Number.isSafeInteger(record.revision) || record.revision < 0) throw new Error("The journal contains a mismatched wallet or network record.");
  const phases = ["prepared", "signed", "unknown", "pending", "included", "finalized", "unresolved", "cancelled"];
  if (!phases.includes(record.phase) || !["unknown", "success", "failed"].includes(record.execution) || !["unseen", "included", "finalized"].includes(record.inclusion)) throw new Error("The journal contains an invalid transaction state.");
  if (record.signed === null) {
    if (!["prepared", "cancelled"].includes(record.phase) || record.execution !== "unknown" || record.inclusion !== "unseen" || record.receipt !== null || record.receipt_proof !== null) throw new Error("The journal contains an impossible unsigned state.");
  } else if (["prepared", "cancelled"].includes(record.phase)) throw new Error("A signed transaction cannot be marked as an unsigned draft.");
  if (record.phase === "finalized" && (record.inclusion !== "finalized" || record.execution === "unknown" || !record.receipt)) throw new Error("The journal contains unsupported finality.");
  if (record.receipt) {
    if (!record.receipt_proof || record.receipt.block_hash !== hash(record.receipt.block_hash) || record.inclusion !== record.receipt.inclusion || record.execution !== record.receipt.execution || record.phase !== record.receipt.phase) throw new Error("The journal contains inconsistent receipt evidence.");
    decimal(record.receipt.actual_fee_wei);
  } else if (record.inclusion !== "unseen" || record.receipt_proof !== null) throw new Error("The journal is missing its inclusion evidence.");
  const expectedNonceKey = `${record.account}:${p.request.nonce}`;
  if (record.phase === "cancelled" ? record.nonce_key !== undefined : record.nonce_key !== expectedNonceKey) throw new Error("The journal nonce reservation is inconsistent.");
}
function assertSnapshot(snapshot: BalanceSnapshot, chain: ChainId, from: string): void {
  if (!snapshot || snapshot.chain_id !== chain || !sameAddress(snapshot.address, from)) throw new Error("Balance belongs to a different wallet or network.");
  accountKey(chain, from); hash(snapshot.block_hash);
  if (!Number.isSafeInteger(snapshot.block_number) || snapshot.block_number < 0 || !Number.isSafeInteger(snapshot.observed_at) || snapshot.observed_at < 0) throw new Error("Invalid balance observation.");
  if (!snapshot.balances || Object.keys(snapshot.balances).sort().join(",") !== "ETH,EURC,USDC") throw new Error("Invalid asset balance set.");
  for (const value of Object.values(snapshot.balances)) decimal(value);
}
interface StoredActivity {
  id: string; account: string; activity: ChainActivity; proof: ActivityProof;
  invalidation: { canonical_hash: string; observed_at: number; finalized_incident: boolean } | null;
}

export class Journal {
  private constructor(private db: IDBDatabase, private core: CoreApi) {}
  static async open(core: CoreApi, factory: IDBFactory = indexedDB, name = DB_NAME): Promise<Journal> {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const open = factory.open(name, 3);
      open.onupgradeneeded = () => {
        const db = open.result;
        if (!db.objectStoreNames.contains("vault")) {
        db.createObjectStore("vault", { keyPath: "id" });
        const payments = db.createObjectStore("payments", { keyPath: "intent_id" });
        payments.createIndex("account", "account");
        payments.createIndex("nonce", "nonce_key", { unique: true });
        const events = db.createObjectStore("events", { keyPath: "id", autoIncrement: true });
        events.createIndex("intent", "intent_id");
        db.createObjectStore("snapshots", { keyPath: "key" });
        db.createObjectStore("latest", { keyPath: "account" });
        }
        if (!db.objectStoreNames.contains("activities")) {
          const activities=db.createObjectStore("activities",{keyPath:"id"});
          activities.createIndex("account","account");
          const events=db.createObjectStore("activity_events",{keyPath:"sequence",autoIncrement:true});
          events.createIndex("activity","id");
          db.createObjectStore("activity_cursors",{keyPath:"account"});
        }
        if (!db.objectStoreNames.contains("protection")) db.createObjectStore("protection", { keyPath: "id" });
      };
      open.onsuccess = () => resolve(open.result);
      open.onerror = () => reject(new Error("Local wallet storage is unavailable. Use a regular browser window with storage enabled."));
      open.onblocked = () => reject(new Error("Close other Local Wallet tabs before opening this version."));
    });
    db.onversionchange = () => db.close();
    const result = new Journal(db, core);
    const tx = result.write(["vault"]);
    await complete(tx);
    return result;
  }
  close(): void { this.db.close(); }
  private write(stores: string[]): IDBTransaction {
    let tx: IDBTransaction;
    try { tx = this.db.transaction(stores, "readwrite", { durability: "strict" }); }
    catch { throw new Error("This browser cannot create a durable wallet journal. Use an up-to-date browser."); }
    if (tx.durability !== "strict") {
      tx.abort();
      throw new Error("This browser does not support strict transaction durability. Sending is disabled.");
    }
    return tx;
  }
  private async atomic<T>(stores: string[], body: (tx: IDBTransaction) => Promise<T>): Promise<T> {
    const tx = this.write(stores);
    const committed = complete(tx);
    try {
      const result = await body(tx);
      await committed;
      return result;
    } catch (error) {
      try { tx.abort(); } catch { /* already completed or aborted */ }
      await committed.catch(() => undefined);
      throw error;
    }
  }
  async vault(): Promise<Vault | null> {
    const value = await request(this.db.transaction("vault").objectStore("vault").get("primary"));
    return value ? validateVault(value) : null;
  }
  async saveVault(vault: Vault): Promise<Vault> {
    validateVault(vault);
    if (vault.protection) throw new Error("A protected wallet must be saved with its device key in one durable transaction.");
    return this.atomic(["vault"], async tx => {
      const store = tx.objectStore("vault");
      const existing = await request(store.get("primary"));
      if (existing) {
        if (constantJson(existing) !== constantJson(vault)) throw new Error("A wallet already exists here. The existing wallet has been preserved.");
        return validateVault(existing);
      }
      await request(store.add(vault));
      return vault;
    });
  }
  private protectionPair(vaultValue: unknown, protectionValue: unknown, expected?: Vault): VaultProtectionState {
    const vault = validateVault(vaultValue);
    if (expected) assertSameVault(vault, expected);
    if (!vault.protection) {
      if (protectionValue) throw new Error("The saved wallet and its protection record do not match. Existing data has been preserved.");
      return { vault, protection: null };
    }
    if (!protectionValue) throw new Error("The device key for this wallet is missing. Existing wallet data has been preserved; restore with your recovery words on another device.");
    return { vault, protection: validateProtection(protectionValue, vault) };
  }
  async protectionState(expected: Vault): Promise<VaultProtectionState> {
    const tx = this.db.transaction(["vault", "protection"]);
    const [vault, protection] = await Promise.all([
      request(tx.objectStore("vault").get("primary")), request(tx.objectStore("protection").get("primary")),
    ]);
    return this.protectionPair(vault, protection, expected);
  }
  async saveProtectedVault(vault: Vault, protection: VaultProtectionRecord, authorized: () => boolean = () => true): Promise<Vault> {
    validateVault(vault); validateProtection(protection, vault);
    if (protection.revision !== 1) throw new Error("A new wallet must begin at protection revision 1.");
    return this.atomic(["vault", "protection"], async tx => {
      const store = tx.objectStore("vault"), keys = tx.objectStore("protection");
      const [existingVault, existingKey] = await Promise.all([request(store.get("primary")), request(keys.get("primary"))]);
      if (existingVault || existingKey) throw new Error("A wallet already exists here. The existing wallet has been preserved.");
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
      await Promise.all([request(store.add(vault)), request(keys.add(protection))]);
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
      return vault;
    });
  }
  async replaceProtection(expectedVault: Vault, expected: VaultProtectionRecord | null, nextVault: Vault, next: VaultProtectionRecord, authorized: () => boolean = () => true): Promise<Vault> {
    validateVault(nextVault); validateProtection(next, nextVault);
    if (!sameAddress(expectedVault.address, nextVault.address) || expectedVault.created_at !== nextVault.created_at
      || nextVault.backup_confirmed !== expectedVault.backup_confirmed || next.revision !== (expected?.revision ?? 0) + 1) throw new Error("A lock change cannot replace the wallet identity or backup state.");
    return this.atomic(["vault", "protection"], async tx => {
      const store = tx.objectStore("vault"), keys = tx.objectStore("protection");
      const [currentVault, currentKey] = await Promise.all([request(store.get("primary")), request(keys.get("primary"))]);
      const current = this.protectionPair(currentVault, currentKey, expectedVault);
      if (expected === null ? current.protection !== null : !current.protection || !sameProtectionPolicy(current.protection, expected)) throw conflict();
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
      // A concurrent backup confirmation is retained; an authentication change never undoes it.
      const saved = { ...nextVault, backup_confirmed: current.vault.backup_confirmed };
      await Promise.all([request(store.put(saved)), request(keys.put(next))]);
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
      return saved;
    });
  }
  async assertProtectionCurrent(expectedVault: Vault, expected: VaultProtectionRecord | null): Promise<void> {
    const current = await this.protectionState(expectedVault);
    if (expected === null ? current.protection !== null : !current.protection || !sameProtectionPolicy(current.protection, expected)) throw conflict();
  }
  async reserveProtectionAttempt(expectedVault: Vault, expected: VaultProtectionRecord, at: number, authorized: () => boolean = () => true): Promise<VaultProtectionAttempt> {
    if (!Number.isSafeInteger(at) || at < 0 || expected.mode !== "pin") throw new Error("Invalid unlock attempt.");
    return this.atomic(["vault", "protection"], async tx => {
      const store = tx.objectStore("protection");
      const [vault, saved] = await Promise.all([request(tx.objectStore("vault").get("primary")), request(store.get("primary"))]);
      const current = this.protectionPair(vault, saved, expectedVault).protection;
      if (!current || !sameProtectionPolicy(current, expected)) throw conflict();
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
      if (current.retry_at > at) throw new Error(`Too many incorrect PIN attempts. Try again in ${Math.ceil((current.retry_at - at) / 1000)} seconds.`);
      if (current.failed_attempts >= Number.MAX_SAFE_INTEGER - 1 || current.attempt_serial >= Number.MAX_SAFE_INTEGER - 1) throw new Error("The saved unlock-attempt counter is invalid.");
      const count = current.failed_attempts + 1;
      const delay = count < 5 ? 0 : Math.min(3_600_000, 30_000 * 2 ** Math.min(count - 5, 7));
      const next = { ...current, failed_attempts: count, retry_at: at + delay, attempt_serial: current.attempt_serial + 1 };
      // Reserve the possible failure before decrypting. A crash or reload cannot erase it.
      await request(store.put(next));
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
      return { revision: next.revision, key_id: next.key_id, serial: next.attempt_serial };
    });
  }
  async completeProtectionAttempt(expectedVault: Vault, expected: VaultProtectionRecord, attempt: VaultProtectionAttempt, authorized: () => boolean = () => true): Promise<void> {
    await this.atomic(["vault", "protection"], async tx => {
      const store = tx.objectStore("protection");
      const [vault, saved] = await Promise.all([request(tx.objectStore("vault").get("primary")), request(store.get("primary"))]);
      const current = this.protectionPair(vault, saved, expectedVault).protection;
      if (!current || !sameProtectionPolicy(current, expected) || current.revision !== attempt.revision
        || current.key_id !== attempt.key_id || current.attempt_serial !== attempt.serial) throw conflict();
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
      await request(store.put({ ...current, failed_attempts: 0, retry_at: 0 }));
      if (!authorized()) throw new Error("Wallet operation was cancelled.");
    });
  }
  async confirmBackup(expected: Vault): Promise<Vault> {
    return this.atomic(["vault"], async tx => {
      const store = tx.objectStore("vault");
      const current = validateVault(await request(store.get("primary")));
      if (current.vault_hex !== expected.vault_hex || !sameAddress(current.address, expected.address) || current.salt !== expected.salt) throw conflict();
      const confirmed = { ...current, backup_confirmed: true };
      await request(store.put(confirmed));
      return confirmed;
    });
  }
  async list(chain: ChainId, from: string): Promise<PaymentRecord[]> {
    const records: PaymentRecord[] = await request(this.db.transaction("payments").objectStore("payments").index("account").getAll(accountKey(chain, from)));
    for (const record of records) {
      assertRecord(record, chain, from);
      await this.core.validatePrepared(record.prepared);
      if (record.signed) await this.core.validateSigned(record.prepared, record.signed);
      await this.validateStoredReceipt(record);
    }
    return records.sort((a, b) => b.created_at - a.created_at || b.intent_id.localeCompare(a.intent_id));
  }
  async get(id: string): Promise<PaymentRecord> {
    const result: PaymentRecord | undefined = await request(this.db.transaction("payments").objectStore("payments").get(id));
    if (!result) throw new Error("The transaction is not in the durable journal.");
    const vault = await this.vault();
    if (!vault) throw new Error("The wallet record is missing.");
    assertRecord(result, result.prepared.request.chain_id, vault.address);
    await this.core.validatePrepared(result.prepared);
    if (result.signed) await this.core.validateSigned(result.prepared, result.signed);
    await this.validateStoredReceipt(result);
    return result;
  }
  private async validateStoredReceipt(record: PaymentRecord): Promise<void> {
    if (!record.receipt) return;
    const proof = record.receipt_proof;
    if (!record.signed || !proof) throw new Error("The journal is missing receipt verification evidence.");
    const checked = await this.core.validateReceipt(record.signed, proof.receipt, proof.canonical_hash, proof.finalized_height);
    if (constantJson(checked) !== constantJson(record.receipt)) throw new Error("Stored receipt evidence does not match the verified transaction outcome.");
  }
  private event(tx: IDBTransaction, id: string, kind: string, evidence: unknown): void {
    const event: JournalEvent = { intent_id: id, time: now(), kind, evidence };
    tx.objectStore("events").add(event);
  }
  async recordPrepared(value: PreparedPayment): Promise<PaymentRecord> {
    const prepared = await this.core.validatePrepared(value);
    const p = prepared.request;
    const account = accountKey(p.chain_id, p.from);
    return this.atomic(["vault", "payments", "events"], async tx => {
      const vault = validateVault(await request(tx.objectStore("vault").get("primary")));
      if (!vault.backup_confirmed || !sameAddress(vault.address, p.from)) throw new Error("Confirm this wallet’s recovery backup before sending.");
      const store = tx.objectStore("payments");
      const existing: PaymentRecord | undefined = await request(store.get(p.intent_id));
      if (existing) {
        if (constantJson(existing.prepared) !== constantJson(prepared)) throw new Error("An intent cannot be overwritten with a different payment.");
        return existing;
      }
      const records: PaymentRecord[] = await request(store.index("account").getAll(account));
      for (const record of records) assertRecord(record, p.chain_id, p.from);
      if (records.some(isActive)) throw new Error("Resolve the existing payment on this network before creating another.");
      const record: PaymentRecord = {
        intent_id: p.intent_id, account, nonce_key: `${account}:${p.nonce}`,
        prepared, signed: null, phase: "prepared", execution: "unknown", inclusion: "unseen",
        receipt: null, receipt_proof: null, resolution_note: null, created_at: prepared.created_at, updated_at: now(), revision: 0,
      };
      await request(store.add(record));
      this.event(tx, record.intent_id, "prepared", prepared);
      return record;
    });
  }
  private async update(expected: PaymentRecord, kind: string, evidence: unknown, change: (current: PaymentRecord) => PaymentRecord): Promise<PaymentRecord> {
    return this.atomic(["payments", "events"], async tx => {
      const store = tx.objectStore("payments");
      const current: PaymentRecord | undefined = await request(store.get(expected.intent_id));
      if (!current || constantJson(current) !== constantJson(expected)) throw conflict();
      const next = change(current);
      if (next === current) return current;
      next.updated_at = now(); next.revision = current.revision + 1;
      assertRecord(next, current.prepared.request.chain_id, current.prepared.request.from);
      await request(store.put(next));
      this.event(tx, next.intent_id, kind, evidence);
      return next;
    });
  }
  async recordSigned(signed: SignedPayment): Promise<PaymentRecord> {
    const current = await this.get(signed.intent_id);
    const validated = await this.core.validateSigned(current.prepared, signed);
    return this.update(current, "signed", validated, record => {
      if (record.signed) {
        if (constantJson(record.signed) !== constantJson(validated)) throw new Error("Different bytes already exist for this payment. No replacement was signed.");
        return record;
      }
      if (record.phase !== "prepared") throw new Error("This draft can no longer be signed.");
      return { ...record, signed: validated, phase: "signed" };
    });
  }
  async cancel(id: string): Promise<PaymentRecord> {
    const current = await this.get(id);
    return this.update(current, "cancelled_unsigned", null, record => {
      if (record.signed) throw new Error("Signed transactions cannot be cancelled by deleting a local record.");
      if (record.phase === "cancelled") return record;
      if (record.phase !== "prepared") throw new Error("Only an unsigned draft can be discarded.");
      const next = { ...record, phase: "cancelled" as const };
      delete next.nonce_key;
      return next;
    });
  }
  async submission(id: string, outcome: "pending" | "unknown"): Promise<PaymentRecord> {
    const current = await this.get(id);
    return this.update(current, "submission_observed", { outcome }, record => {
      if (!record.signed) throw new Error("An unsigned payment cannot be broadcast.");
      if (["included", "finalized", "unresolved"].includes(record.phase) || record.phase === outcome || (record.phase === "pending" && outcome === "unknown")) return record;
      return { ...record, phase: outcome };
    });
  }
  async observe(id: string, rawReceipt: unknown, canonicalHash: string, finalizedHeight: number): Promise<PaymentRecord> {
    const current = await this.get(id);
    if (!current.signed) throw new Error("Cannot apply a receipt to an unsigned draft.");
    const receipt = await this.core.validateReceipt(current.signed, rawReceipt, canonicalHash, finalizedHeight);
    const proof = { receipt: rawReceipt, canonical_hash: canonicalHash, finalized_height: finalizedHeight };
    return this.update(current, "receipt_observed", { receipt, proof }, record => {
      if (record.receipt) {
        if (record.receipt.block_hash !== receipt.block_hash) throw new Error("Record a reorganization before accepting different inclusion evidence.");
        if (record.receipt.receipt_fingerprint !== receipt.receipt_fingerprint) throw new Error("Conflicting receipt evidence for the same canonical block. Sending remains blocked.");
        if (record.inclusion === "finalized" || constantJson(record.receipt) === constantJson(receipt)) return record;
      }
      return { ...record, receipt, receipt_proof: proof, phase: receipt.phase, execution: receipt.execution, inclusion: receipt.inclusion, resolution_note: receipt.resolution_note };
    });
  }
  async reorg(id: string, previousBlockHash: string): Promise<PaymentRecord> {
    const current = await this.get(id);
    return this.update(current, "inclusion_invalidated", { previous_block_hash: previousBlockHash }, record => {
      if (!record.receipt || record.receipt.block_hash !== hash(previousBlockHash)) throw conflict();
      const finalized = record.inclusion === "finalized";
      return { ...record, receipt: null, receipt_proof: null, phase: finalized ? "unresolved" : "unknown", execution: "unknown", inclusion: "unseen", resolution_note: finalized ? "Previously finalized inclusion is no longer canonical. Stop and investigate the network evidence." : "The earlier block was reorganized. Submission outcome is being reconciled." };
    });
  }
  async recordBalance(snapshot: BalanceSnapshot): Promise<BalanceSnapshot> {
    const account = accountKey(snapshot.chain_id, snapshot.address);
    assertSnapshot(snapshot, snapshot.chain_id, snapshot.address);
    const key = `${account}:${snapshot.block_hash}`;
    return this.atomic(["vault", "snapshots", "latest"], async tx => {
      const vault = validateVault(await request(tx.objectStore("vault").get("primary")));
      if (!sameAddress(vault.address, snapshot.address)) throw new Error("Balance belongs to another wallet.");
      const store = tx.objectStore("snapshots");
      const existing = await request(store.get(key));
      const latest = await request(tx.objectStore("latest").get(account));
      if (latest) assertSnapshot(latest.snapshot, snapshot.chain_id, snapshot.address);
      if (existing) {
        const old = existing.snapshot as BalanceSnapshot;
        assertSnapshot(old, snapshot.chain_id, snapshot.address);
        if (old.block_number !== snapshot.block_number || constantJson(old.balances) !== constantJson(snapshot.balances)) throw new Error("The provider returned conflicting balances for the same block.");
        // An old idempotent result must never promote stale balances.
        return latest?.snapshot ?? old;
      }
      if (latest && latest.snapshot.block_number > snapshot.block_number) throw new Error("The provider returned stale balances. The newer observation was preserved.");
      await request(store.add({ key, account, snapshot }));
      await request(tx.objectStore("latest").put({ account, snapshot }));
      return snapshot;
    });
  }
  async latestBalance(chain: ChainId, from: string): Promise<BalanceSnapshot | null> {
    const account = accountKey(chain, from);
    const tx = this.db.transaction(["latest", "snapshots"]);
    const entry = await request(tx.objectStore("latest").get(account));
    if (!entry) return null;
    if (entry.account !== account) throw new Error("The balance index belongs to a different account.");
    assertSnapshot(entry.snapshot, chain, from);
    const stored = await request(tx.objectStore("snapshots").get(`${account}:${entry.snapshot.block_hash}`));
    if (!stored || stored.account !== account || constantJson(stored.snapshot) !== constantJson(entry.snapshot)) throw new Error("The balance projection is not supported by its recorded observation.");
    return entry.snapshot;
  }
  async events(id: string): Promise<JournalEvent[]> {
    await this.get(id);
    return request(this.db.transaction("events").objectStore("events").index("intent").getAll(id));
  }
  private async verifyActivity(entry: StoredActivity, chain: ChainId, from: string): Promise<void> {
    if (!this.core.validateActivity) throw new Error("Reload to complete the wallet’s activity-validator update.");
    if (!entry || entry.account!==accountKey(chain,from) || entry.id!==entry.activity.id || entry.activity.chain_id!==chain || !sameAddress(entry.activity.address,from)) throw new Error("Activity belongs to a different wallet or network.");
    const verified=await this.core.validateActivity(chain,from,entry.proof);
    const original=verified.activities.find(item=>item.id===entry.id);
    if (!original) throw new Error("Stored activity is not supported by its transaction receipt.");
    if (entry.invalidation) {
      if (entry.activity.inclusion!=="reorged" || hash(entry.invalidation.canonical_hash)===entry.activity.block_hash || entry.invalidation.finalized_incident!==(original.inclusion==="finalized") || constantJson({...entry.activity,inclusion:original.inclusion})!==constantJson(original)) throw new Error("Stored activity reorganization evidence is inconsistent.");
    } else if (constantJson(original)!==constantJson(entry.activity)) throw new Error("Stored activity does not match its verified receipt.");
  }
  async listActivity(chain: ChainId, from: string): Promise<ChainActivity[]> {
    const entries:StoredActivity[]=await request(this.db.transaction("activities").objectStore("activities").index("account").getAll(accountKey(chain,from)));
    for(const entry of entries) await this.verifyActivity(entry,chain,from);
    return entries.map(entry=>entry.activity).sort((a,b)=>b.block_number-a.block_number || (b.log_index??-1)-(a.log_index??-1));
  }
  async transactionActivity(chain: ChainId, from: string, transactionHash: string): Promise<ChainActivity[]> {
    const txHash=hash(transactionHash);
    const entries:StoredActivity[]=await request(this.db.transaction("activities").objectStore("activities").index("account").getAll(accountKey(chain,from)));
    const matching=entries.filter(entry=>entry.activity.transaction_hash===txHash);
    for(const entry of matching) await this.verifyActivity(entry,chain,from);
    return matching.map(entry=>entry.activity);
  }
  async recordActivity(chain: ChainId, from: string, proof: ActivityProof): Promise<ChainActivity[]> {
    if(!this.core.validateActivity) throw new Error("Reload to complete the wallet’s activity-validator update.");
    const validated=await this.core.validateActivity(chain,from,proof);
    // Each retained projection carries its proof. Bound that multiplication,
    // not just the size of an individual RPC response, before any write.
    if(new TextEncoder().encode(JSON.stringify(proof)).byteLength*validated.activities.length>4_194_304) throw new Error("This transaction has too much activity evidence to record here. Its on-chain balance remains independent; inspect the transaction in the explorer.");
    const account=accountKey(chain,from);
    return this.atomic(["vault","activities","activity_events"],async tx=>{
      const vault=validateVault(await request(tx.objectStore("vault").get("primary")));
      if(!sameAddress(vault.address,from)) throw new Error("Activity belongs to another wallet.");
      const store=tx.objectStore("activities");
      const existing:StoredActivity[]=await request(store.index("account").getAll(account));
      const result:ChainActivity[]=[];
      for(const txHash of new Set(validated.activities.map(item=>item.transaction_hash))) {
        const batch=validated.activities.filter(item=>item.transaction_hash===txHash);
        const prior=existing.filter(entry=>entry.activity.transaction_hash===txHash && entry.activity.block_hash===batch[0].block_hash && !entry.invalidation);
        if(prior.length && constantJson(prior.map(entry=>entry.id).sort())!==constantJson(batch.map(item=>item.id).sort())) throw new Error("This receipt changes the transfer set already observed in the same block. The earlier evidence is preserved.");
      }
      for(const activity of validated.activities) {
        const related=existing.filter(entry=>entry.activity.transaction_hash===activity.transaction_hash);
        if(related.some(entry=>entry.activity.block_hash!==activity.block_hash && !entry.invalidation)) throw new Error("The prior transaction inclusion must be explicitly invalidated before another block is accepted.");
        const incidents=related.filter(entry=>entry.invalidation?.finalized_incident);
        if(incidents.length) {
          for(const incident of incidents) if(!result.some(item=>item.id===incident.id)) result.push(incident.activity);
          continue;
        }
        const previous:StoredActivity|undefined=await request(store.get(activity.id));
        if(previous) {
          if(previous.account!==account) throw new Error("Activity identity conflict.");
          if(previous.activity.block_hash!==activity.block_hash && !previous.invalidation) throw new Error("The prior activity inclusion must be explicitly invalidated before accepting another block.");
          if(previous.activity.block_hash===activity.block_hash) {
            if(previous.invalidation) throw new Error("A previously invalidated block cannot silently become canonical again.");
            if(constantJson({...previous.activity,inclusion:activity.inclusion})!==constantJson(activity)) throw new Error("Conflicting activity evidence for the same transaction and block.");
            if(previous.activity.inclusion==="finalized" || previous.activity.inclusion===activity.inclusion) {result.push(previous.activity);continue;}
          }
        }
        const next:StoredActivity={id:activity.id,account,activity,proof,invalidation:null};
        await request(store.put(next));
        tx.objectStore("activity_events").add({id:activity.id,account,kind:"observed",observed_at:now(),evidence:next});
        result.push(activity);
      }
      return result;
    });
  }
  async invalidateActivity(chain: ChainId, from: string, id: string, previousBlockHash: string, canonicalHash: string): Promise<void> {
    const account=accountKey(chain,from);const oldHash=hash(previousBlockHash);const nextHash=hash(canonicalHash);
    if(oldHash===nextHash) throw new Error("Canonical activity cannot be invalidated.");
    await this.atomic(["activities","activity_events"],async tx=>{
      const store=tx.objectStore("activities");const previous:StoredActivity|undefined=await request(store.get(id));
      if(!previous || previous.account!==account || previous.activity.block_hash!==oldHash) throw conflict();
      if(previous.invalidation) return;
      const next:StoredActivity={...previous,activity:{...previous.activity,inclusion:"reorged"},invalidation:{canonical_hash:nextHash,observed_at:now(),finalized_incident:previous.activity.inclusion==="finalized"}};
      await request(store.put(next));
      tx.objectStore("activity_events").add({id,account,kind:"inclusion_invalidated",observed_at:now(),evidence:{previous,next}});
    });
  }
  async activityCursor(chain: ChainId, from: string): Promise<number|null> {
    const value=await request(this.db.transaction("activity_cursors").objectStore("activity_cursors").get(accountKey(chain,from)));
    if(!value) return null;
    if(value.account!==accountKey(chain,from) || !Number.isSafeInteger(value.height) || value.height<0) throw new Error("Invalid activity scan cursor.");
    return value.height;
  }
  async recordActivityCursor(chain: ChainId, from: string, height: number): Promise<void> {
    if(!Number.isSafeInteger(height)||height<0) throw new Error("Invalid activity scan height.");
    await this.atomic(["activity_cursors"],async tx=>{await request(tx.objectStore("activity_cursors").put({account:accountKey(chain,from),height}));});
  }
}
