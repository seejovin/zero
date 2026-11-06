import type { Vault, VaultProtectionRecord } from "./types";
import { constantJson, validateVault } from "./units";

/** Backup confirmation is independently updated and does not re-key the vault. */
export function vaultBinding(vault: Vault): string {
  validateVault(vault);
  return constantJson({ domain: "LocalWalletWebVault/v1", address: vault.address.toLowerCase(),
    version: vault.version, kdf: vault.kdf, salt: vault.salt,
    vault_hex: vault.vault_hex, created_at: vault.created_at });
}

export function assertSameVault(current: Vault, expected: Vault): void {
  if (vaultBinding(current) !== vaultBinding(expected) || current.protection !== expected.protection) {
    throw new Error("The wallet changed in another operation. Reopen it before continuing.");
  }
}

const recordKeys = ["id", "version", "mode", "revision", "key_id", "vault_binding", "device_key", "iv", "ciphertext", "pin_salt", "failed_attempts", "retry_at", "attempt_serial"].sort().join(",");
export function validateProtection(value: unknown, vault: Vault): VaultProtectionRecord {
  const record = value as VaultProtectionRecord;
  const key = record?.device_key;
  const validInteger = (n: unknown): n is number => Number.isSafeInteger(n) && Number(n) >= 0;
  if (!record || Object.keys(record).sort().join(",") !== recordKeys || record.id !== "primary" || record.version !== 1
    || vault.protection !== "device-key-v1" || record.vault_binding !== vaultBinding(vault)
    || !["off", "pin"].includes(record.mode) || !validInteger(record.revision) || record.revision < 1
    || !/^[0-9a-f]{32}$/.test(record.key_id) || !/^[0-9a-f]{24}$/.test(record.iv)
    || !/^[0-9a-f]{32,12288}$/.test(record.ciphertext) || record.ciphertext.length % 2 !== 0
    || (record.mode === "pin" ? !/^[0-9a-f]{32}$/.test(record.pin_salt ?? "") : record.pin_salt !== null)
    || !validInteger(record.failed_attempts) || !validInteger(record.retry_at) || !validInteger(record.attempt_serial)
    || (record.mode === "off" && (record.failed_attempts !== 0 || record.retry_at !== 0 || record.attempt_serial !== 0))
    || !key || key.type !== "secret" || key.extractable !== false || key.algorithm?.name !== "AES-GCM"
    || (key.algorithm as AesKeyAlgorithm).length !== 256 || [...key.usages].sort().join(",") !== "decrypt,encrypt") {
    throw new Error("The saved wallet protection could not be validated. Existing wallet data has been preserved.");
  }
  return record;
}

/** AEAD authenticates policy and identity; mutable attempt counters are separate. */
export function protectionAad(record: VaultProtectionRecord): Uint8Array<ArrayBuffer> {
  return new TextEncoder().encode(constantJson({ domain: "LocalWalletWebProtection/v1", mode: record.mode,
    revision: record.revision, key_id: record.key_id, vault_binding: record.vault_binding, pin_salt: record.pin_salt }));
}

export function sameProtectionPolicy(left: VaultProtectionRecord, right: VaultProtectionRecord): boolean {
  return left.revision === right.revision && left.key_id === right.key_id && left.vault_binding === right.vault_binding
    && left.mode === right.mode && left.iv === right.iv && left.ciphertext === right.ciphertext && left.pin_salt === right.pin_salt;
}
