import type { Asset, CoreApi, PaymentRecord, PreparedPayment, Vault } from "./types";
import { Journal, isActive } from "./journal";
import { NetworkClient } from "./network";
import { ActivityReader } from "./activity";
import { constantJson, hash, hexQuantity, quantity, safeNumber, validatePreparedIdentity } from "./units";

export interface SubmissionResult { record: PaymentRecord; acknowledged: boolean }
export type AuthorizationCheck = () => boolean | Promise<boolean>;
export class WalletEngine {
  constructor(public readonly core: CoreApi, public readonly journal: Journal, public readonly rpc: NetworkClient) {}
  refreshActivity(address: string) { return new ActivityReader(this.core,this.journal,this.rpc).refresh(address); }
  lookupTransaction(address: string, transactionHash: string) { return new ActivityReader(this.core,this.journal,this.rpc).lookup(address,transactionHash); }
  async prepare(vault: Vault, to: string, asset: Asset, amount: string): Promise<PreparedPayment> {
    if (!vault.backup_confirmed) throw new Error("Confirm the recovery backup before sending.");
    const records = await this.journal.list(this.rpc.network.chain_id, vault.address);
    if (records.some(isActive)) throw new Error("Resolve the existing payment on this network before creating another.");
    const quote = await this.rpc.quote(crypto.randomUUID(), vault.address, to, asset, amount);
    const prepared = await this.core.prepare(quote.request);
    validatePreparedIdentity(prepared, vault, this.rpc.network.chain_id);
    await this.journal.recordBalance(quote.snapshot);
    await this.journal.recordPrepared(prepared);
    return prepared;
  }
  async signAndSubmit(vault: Vault, password: string, reviewed: PreparedPayment, stillAuthorized: AuthorizationCheck = () => true): Promise<SubmissionResult> {
    validatePreparedIdentity(reviewed, vault, this.rpc.network.chain_id);
    const current = await this.journal.get(reviewed.request.intent_id);
    if (current.phase !== "prepared" || current.signed !== null || constantJson(current.prepared) !== constantJson(reviewed)) throw new Error("The payment changed or was already signed. Refresh its history.");
    await this.rpc.revalidate(reviewed);
    // The app also checks the durable protection revision here. A promise must
    // resolve before signing; cross-tab cancellation cannot rely on UI events.
    if (!(await stillAuthorized())) throw new Error("Wallet locked before signing.");
    let signed;
    try { signed = await this.core.sign(vault, password, reviewed); }
    finally { password = ""; }
    if (!(await stillAuthorized())) throw new Error("Wallet locked. No transaction was submitted.");
    // An IndexedDB request's success is NOT a commit. recordSigned resolves only
    // after the strict-durability transaction's complete event.
    await this.journal.recordSigned(signed);
    // The network receives only data read back from the durable journal.
    const durable = await this.journal.get(signed.intent_id);
    if (!(await stillAuthorized())) throw new Error("Wallet locked. Signed data is saved; check transaction history before retrying.");
    return this.submitDurable(durable, stillAuthorized);
  }
  private async submitDurable(record: PaymentRecord, stillAuthorized: AuthorizationCheck): Promise<SubmissionResult> {
    if (!record.signed || !["signed", "unknown", "pending"].includes(record.phase)) throw new Error("This transaction is not eligible for submission.");
    await this.rpc.verifyChain();
    if (!(await stillAuthorized())) throw new Error("Wallet locked. Signed data is saved; check transaction history before retrying.");
    let acknowledged = false;
    try {
      await this.rpc.broadcast(record.signed);
      acknowledged = true;
    } catch {
      // A timeout, HTTP error, RPC error, or wrong returned hash cannot prove
      // that broadcast failed. Do not allocate a new nonce or sign new bytes.
    }
    const saved = await this.journal.submission(record.intent_id, acknowledged ? "pending" : "unknown");
    return { record: saved, acknowledged };
  }
  async retry(id: string, stillAuthorized: AuthorizationCheck = () => true): Promise<SubmissionResult> {
    const record = await this.reconcile(id);
    if (["included", "finalized", "unresolved"].includes(record.phase)) return { record, acknowledged: record.phase !== "unresolved" };
    return this.submitDurable(await this.journal.get(id), stillAuthorized);
  }
  async reconcile(id: string): Promise<PaymentRecord> {
    let record = await this.journal.get(id);
    if (!record.signed) return record;
    if (record.phase === "unresolved" && record.inclusion === "unseen") throw new Error("Previously finalized evidence was contradicted. Stop sending and investigate this account.");
    const signed = record.signed;
    if (signed.chain_id !== this.rpc.network.chain_id) throw new Error("The journal payment belongs to another network.");
    await this.rpc.verifyChain();
    const receipt = await this.rpc.receipt(signed);
    if (record.receipt) {
      // A conflicting NEW receipt cannot invalidate OLD evidence. Check the
      // old inclusion at its own recorded height first.
      const previousBlock = await this.rpc.block(hexQuantity(record.receipt.block_number));
      if (hash(previousBlock.hash) !== record.receipt.block_hash) {
        record = await this.journal.reorg(id, record.receipt.block_hash);
        if (record.phase === "unresolved" || receipt === null) return record;
      } else if (receipt !== null && hash(receipt.blockHash) !== record.receipt.block_hash) {
        throw new Error("The provider returned conflicting inclusion while the earlier block remains canonical. Earlier evidence is preserved.");
      }
    }
    if (receipt === null) {
      if (record.receipt) {
        // A missing receipt at a provider does not invalidate a canonical block.
        throw new Error("The provider temporarily omitted this receipt. Its earlier evidence has been preserved.");
      }
      const present = await this.rpc.transactionPresent(signed);
      return this.journal.submission(id, present ? "pending" : "unknown");
    }
    const height = safeNumber(quantity(receipt.blockNumber));
    const canonical = await this.rpc.block(hexQuantity(height));
    if (hash(canonical.hash) !== hash(receipt.blockHash)) {
      throw new Error("The receipt is not in the current canonical block. Outcome remains unresolved.");
    }
    const normalized = await this.rpc.receiptFees(receipt);
    const finalized = await this.rpc.block("finalized");
    // The finalized lookup may race a reorganization. Read the receipt's block
    // once more after that lookup before applying any finality assertion.
    await this.rpc.assertCanonical(canonical);
    return this.journal.observe(id, normalized, hash(canonical.hash), safeNumber(quantity(finalized.number)));
  }
}
