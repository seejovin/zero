import { Journal } from "./journal";
import type { CoreApi, Vault, VaultProtectionAttempt, VaultProtectionRecord, VaultProtectionState } from "./types";
import { protectionAad, validateProtection, vaultBinding } from "./protection-format";
import { sameAddress, validateVault } from "./units";

export interface WalletLockStatus {
  mode: "off" | "pin" | "password";
  retry_at: number;
  failed_attempts: number;
}

export function validateAppPin(pin: string): void {
  if (!/^[0-9]{6}$/.test(pin)) throw new Error("Use exactly 6 digits for your app PIN.");
}
function assertAuthorized(authorized: () => boolean): void {
  if (!authorized()) throw new Error("Wallet operation was cancelled.");
}
function hex(bytes: Uint8Array): string {
  return Array.from(bytes, value => value.toString(16).padStart(2, "0")).join("");
}
function fromHex(value: string): Uint8Array<ArrayBuffer> {
  if (!/^(?:[0-9a-f]{2})+$/.test(value)) throw new Error("Invalid encrypted wallet protection.");
  return Uint8Array.from(value.match(/../g)!, pair => parseInt(pair, 16));
}

/**
 * An optional application lock, separate from account derivation/recovery words.
 *
 * Managed vaults use a random 256-bit core password. Its envelope is protected
 * by a non-extractable WebCrypto key persisted through IndexedDB structured
 * cloning. A PIN adds an Argon2id/AES-GCM inner envelope. It is not a raw PIN
 * verifier or a UI-only lock. The core still validates/decrypts every operation.
 *
 * WebCrypto non-extractability does not promise hardware-backed storage, protect
 * against malicious same-origin script, or stop rollback of a stolen profile.
 * With app lock off, anyone able to use the browser profile can use the wallet.
 * JS strings cannot be reliably zeroized; callers must not retain credentials.
 */
export class WalletProtection {
  constructor(private core: CoreApi, private journal: Journal, private clock: () => number = Date.now,
    private cryptography: Crypto = crypto) {}

  async status(vault: Vault): Promise<WalletLockStatus> {
    const state = await this.journal.protectionState(vault);
    return state.protection
      ? { mode: state.protection.mode, retry_at: state.protection.retry_at, failed_attempts: state.protection.failed_attempts }
      : { mode: "password", retry_at: 0, failed_attempts: 0 };
  }

  private randomHex(size: number): string {
    const bytes = this.cryptography.getRandomValues(new Uint8Array(size));
    try { return hex(bytes); } finally { bytes.fill(0); }
  }
  private async pinKey(pin: string, salt: string): Promise<CryptoKey> {
    validateAppPin(pin);
    if (!this.core.deriveProtectionKey) throw new Error("App-PIN protection is unavailable in this wallet build.");
    const raw = await this.core.deriveProtectionKey(pin, salt);
    try {
      if (!(raw instanceof Uint8Array) || raw.byteLength !== 32) throw new Error("The app-PIN key could not be derived safely.");
      // Copy into a plain ArrayBuffer because WebCrypto must not accept shared memory.
      const bytes = new Uint8Array(raw);
      try { return await this.cryptography.subtle.importKey("raw", bytes, { name: "AES-GCM" }, false, ["encrypt", "decrypt"]); }
      finally { bytes.fill(0); }
    } finally { raw.fill(0); }
  }
  private async protect(vault: Vault, secret: string, pin: string | null, revision: number): Promise<VaultProtectionRecord> {
    if (!/^[0-9a-f]{64}$/.test(secret)) throw new Error("The local vault credential is invalid.");
    if (pin !== null) validateAppPin(pin);
    const key = await this.cryptography.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
    const record: VaultProtectionRecord = {
      id: "primary", version: 1, mode: pin === null ? "off" : "pin", revision,
      key_id: this.randomHex(16), vault_binding: vaultBinding(vault), device_key: key,
      iv: this.randomHex(12), ciphertext: "", pin_salt: pin === null ? null : this.randomHex(16),
      failed_attempts: 0, retry_at: 0, attempt_serial: 0,
    };
    const aad = protectionAad(record);
    let plaintext: Uint8Array<ArrayBuffer> | null = null;
    let innerPlaintext: Uint8Array<ArrayBuffer> | null = null;
    try {
      if (pin === null) {
        plaintext = new TextEncoder().encode(JSON.stringify({ version: 1, secret }));
      } else {
        const innerKey = await this.pinKey(pin, record.pin_salt!);
        const iv = this.randomHex(12);
        innerPlaintext = new TextEncoder().encode(secret);
        const ciphertext = await this.cryptography.subtle.encrypt({ name: "AES-GCM", iv: fromHex(iv), additionalData: aad, tagLength: 128 }, innerKey, innerPlaintext);
        plaintext = new TextEncoder().encode(JSON.stringify({ version: 1, iv, ciphertext: hex(new Uint8Array(ciphertext)) }));
      }
      const encrypted = await this.cryptography.subtle.encrypt({ name: "AES-GCM", iv: fromHex(record.iv), additionalData: aad, tagLength: 128 }, key, plaintext);
      record.ciphertext = hex(new Uint8Array(encrypted));
      return validateProtection(record, vault);
    } finally {
      plaintext?.fill(0); innerPlaintext?.fill(0); secret = ""; pin = null;
    }
  }
  private async unprotect(record: VaultProtectionRecord, pin: string): Promise<string> {
    const aad = protectionAad(record);
    let plaintext: Uint8Array<ArrayBuffer> | null = null;
    let innerPlaintext: Uint8Array<ArrayBuffer> | null = null;
    let payload: Record<string, unknown> | null = null;
    try {
      try {
        plaintext = new Uint8Array(await this.cryptography.subtle.decrypt({ name: "AES-GCM", iv: fromHex(record.iv), additionalData: aad, tagLength: 128 }, record.device_key, fromHex(record.ciphertext)));
        payload = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(plaintext));
      } catch { throw new Error("The device protection for this wallet could not be opened. Existing wallet data has been preserved."); }
      if (!payload || payload.version !== 1) throw new Error("Invalid saved wallet protection.");
      let secret: string;
      if (record.mode === "off") {
        if (Object.keys(payload).sort().join(",") !== "secret,version" || typeof payload.secret !== "string") throw new Error("Invalid saved wallet protection.");
        secret = payload.secret;
      } else {
        if (Object.keys(payload).sort().join(",") !== "ciphertext,iv,version" || typeof payload.iv !== "string" || !/^[0-9a-f]{24}$/.test(payload.iv)
          || typeof payload.ciphertext !== "string" || !/^[0-9a-f]{160}$/.test(payload.ciphertext)) throw new Error("Invalid saved app-PIN protection.");
        const key = await this.pinKey(pin, record.pin_salt!);
        try {
          innerPlaintext = new Uint8Array(await this.cryptography.subtle.decrypt({ name: "AES-GCM", iv: fromHex(payload.iv), additionalData: aad, tagLength: 128 }, key, fromHex(payload.ciphertext)));
          secret = new TextDecoder("utf-8", { fatal: true }).decode(innerPlaintext);
        } catch { throw new Error("Incorrect 6-digit PIN, or the saved wallet protection is damaged."); }
      }
      if (!/^[0-9a-f]{64}$/.test(secret)) throw new Error("Invalid local wallet credential.");
      return secret;
    } finally {
      plaintext?.fill(0); innerPlaintext?.fill(0); pin = "";
      if (payload) { payload.secret = ""; payload.ciphertext = ""; }
    }
  }
  private async unlock(state: VaultProtectionState, input: string, authorized: () => boolean): Promise<string> {
    assertAuthorized(authorized);
    const policy = state.protection;
    let secret = "";
    let attempt: VaultProtectionAttempt | null = null;
    try {
      if (policy?.mode === "pin") {
        validateAppPin(input);
        attempt = await this.journal.reserveProtectionAttempt(state.vault, policy, this.clock(), authorized);
      }
      secret = policy ? await this.unprotect(policy, input) : input;
      assertAuthorized(authorized);
      const opened = await this.core.authenticate(state.vault, secret);
      if (!sameAddress(opened, state.vault.address)) throw new Error("The wallet credential belongs to a different account.");
      assertAuthorized(authorized);
      if (attempt && policy) await this.journal.completeProtectionAttempt(state.vault, policy, attempt, authorized);
      else await this.journal.assertProtectionCurrent(state.vault, policy);
      assertAuthorized(authorized);
      return secret;
    } finally { secret = ""; input = ""; }
  }
  /** The returned password is transient, already authenticated, and never a PIN. */
  async credentials(vault: Vault, userInput = "", authorized: () => boolean = () => true): Promise<string> {
    assertAuthorized(authorized);
    return this.unlock(await this.journal.protectionState(vault), userInput, authorized);
  }
  async create(authorized: () => boolean = () => true): Promise<{ vault: Vault; mnemonic: string }> {
    assertAuthorized(authorized);
    let secret = this.randomHex(32);
    let created: { vault: Vault; mnemonic: string } | null = null;
    try {
      created = await this.core.create(secret);
      assertAuthorized(authorized);
      const vault: Vault = { ...validateVault(created.vault), protection: "device-key-v1" };
      const policy = await this.protect(vault, secret, null, 1);
      assertAuthorized(authorized);
      const saved = await this.journal.saveProtectedVault(vault, policy, authorized);
      return { vault: saved, mnemonic: created.mnemonic };
    } finally { secret = ""; if (created) created.mnemonic = ""; }
  }
  async restore(words: string, authorized: () => boolean = () => true): Promise<Vault> {
    assertAuthorized(authorized);
    let secret = this.randomHex(32);
    try {
      const vault: Vault = { ...validateVault(await this.core.restore(secret, words)), protection: "device-key-v1" };
      assertAuthorized(authorized);
      const policy = await this.protect(vault, secret, null, 1);
      assertAuthorized(authorized);
      return await this.journal.saveProtectedVault(vault, policy, authorized);
    } finally { secret = ""; words = ""; }
  }
  /** Explicit settings operation. Current input is the app PIN or legacy password. */
  async setLock(vault: Vault, currentUserInput: string, nextPin: string | null, authorized: () => boolean = () => true): Promise<Vault> {
    assertAuthorized(authorized);
    if (nextPin !== null) validateAppPin(nextPin);
    const state = await this.journal.protectionState(vault);
    let secret = "", words = "";
    try {
      secret = await this.unlock(state, currentUserInput, authorized);
      let nextVault: Vault = { ...state.vault, protection: "device-key-v1" };
      if (!state.protection) {
        // A legacy password is revoked for the current core ciphertext, not just
        // hidden behind a new PIN dialog. Keep the same seed/account/history.
        words = await this.core.reveal(state.vault, secret);
        assertAuthorized(authorized);
        secret = this.randomHex(32);
        const restored = validateVault(await this.core.restore(secret, words));
        if (!sameAddress(restored.address, state.vault.address)) throw new Error("The restored account does not match the saved wallet. No data was replaced.");
        nextVault = { ...restored, created_at: state.vault.created_at, backup_confirmed: state.vault.backup_confirmed, protection: "device-key-v1" };
      }
      assertAuthorized(authorized);
      const next = await this.protect(nextVault, secret, nextPin, (state.protection?.revision ?? 0) + 1);
      assertAuthorized(authorized);
      return await this.journal.replaceProtection(state.vault, state.protection, nextVault, next, authorized);
    } finally { secret = ""; words = ""; currentUserInput = ""; nextPin = null; }
  }
}
