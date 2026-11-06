import type { CoreApi, Vault, PaymentRequest, PreparedPayment, SignedPayment, ReceiptValidation, Network, ActivityProof, ActivityValidation, ChainId } from "./types";

export class CoreClient implements CoreApi {
  private workers = new Map<Worker, () => void>();
  private call<T>(operation: string, data: unknown = {}): Promise<T> {
    return new Promise((resolve, reject) => {
      const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module", name: "local-wallet-crypto" });
      let settled = false;
      const finish = () => { settled = true; clearTimeout(timer); worker.terminate(); this.workers.delete(worker); };
      const cancel = () => { if (!settled) { finish(); reject(new Error("Wallet locked; operation stopped.")); } };
      const timer = setTimeout(() => { finish(); reject(new Error("The local wallet operation timed out.")); }, 120_000);
      this.workers.set(worker, cancel);
      worker.onmessage = event => {
        finish();
        if (event.data?.ok === true) resolve(event.data.value as T);
        else reject(new Error(typeof event.data?.error === "string" ? event.data.error : "The wallet operation failed."));
      };
      worker.onerror = () => { finish(); reject(new Error("The wallet cryptography module could not run. Reload the application.")); };
      worker.postMessage({ operation, data });
    });
  }
  cancelAll(): void { for (const cancel of [...this.workers.values()]) cancel(); }
  registry(): Promise<{ networks: Network[] }> { return this.call("registry"); }
  create(password: string): Promise<{ vault: Vault; mnemonic: string }> { return this.call("create", { password }); }
  restore(password: string, mnemonic: string): Promise<Vault> { return this.call("restore", { password, mnemonic }); }
  authenticate(vault: Vault, password: string): Promise<string> { return this.call("authenticate", { vault, password }); }
  reveal(vault: Vault, password: string): Promise<string> { return this.call("reveal", { vault, password }); }
  deriveProtectionKey(pin: string, salt: string): Promise<Uint8Array> { return this.call("deriveProtectionKey", { password: pin, salt }); }
  prepare(request: PaymentRequest): Promise<PreparedPayment> { return this.call("prepare", { request }); }
  sign(vault: Vault, password: string, prepared: PreparedPayment): Promise<SignedPayment> { return this.call("sign", { vault, password, prepared }); }
  validatePrepared(prepared: PreparedPayment): Promise<PreparedPayment> { return this.call("validatePrepared", { prepared }); }
  validateSigned(prepared: PreparedPayment, signed: SignedPayment): Promise<SignedPayment> { return this.call("validateSigned", { prepared, signed }); }
  validateReceipt(signed: SignedPayment, receipt: unknown, canonicalHash: string, finalizedHeight: number): Promise<ReceiptValidation> { return this.call("validateReceipt", { signed, receipt, canonicalHash, finalizedHeight }); }
  validateActivity(chain: ChainId, address: string, proof: ActivityProof): Promise<ActivityValidation> { return this.call("validateActivity", { chain, address, proof }); }
}
