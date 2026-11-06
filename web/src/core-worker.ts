// A new worker is used for each operation and terminated after its response.
// Secrets never leave this device through a network request. JS strings cannot
// be reliably zeroized; destruction of the worker bounds their live lifetime.
import init, * as wasm from "./crypto/wallet_core.js";
import { sameAddress, validateVault } from "./units";
import type { Vault, PreparedPayment, SignedPayment, ActivityProof, ChainId } from "./types";

type Payload = {
  password?: string; mnemonic?: string; vault?: Vault; prepared?: PreparedPayment;
  signed?: SignedPayment; request?: unknown; receipt?: unknown; canonicalHash?: string;
  finalizedHeight?: number;
  chain?: ChainId; address?: string; proof?: ActivityProof;
  salt?: string;
};
const secretOperations = new Set(["create", "restore", "authenticate", "reveal", "sign", "deriveProtectionKey"]);
function fromHex(hex: string): Uint8Array {
  return Uint8Array.from(hex.match(/.{2}/g) ?? [], pair => parseInt(pair, 16));
}
function keyFor(vault: Vault, password: string): Uint8Array {
  validateVault(vault);
  return wasm.derive_vault_key(password, fromHex(vault.salt));
}
function verifyAddress(vault: Vault, key: Uint8Array): string {
  const result = wasm.wallet_address(vault.vault_hex, key);
  if (!sameAddress(result, vault.address)) throw new Error("Wallet metadata does not match its encrypted vault.");
  return result;
}
function run(operation: string, data: Payload): unknown {
  let key: Uint8Array | null = null;
  try {
    switch (operation) {
      case "registry": return JSON.parse(wasm.asset_registry());
      case "deriveProtectionKey": {
        if (!/^[0-9]{6}$/.test(data.password ?? "") || !/^[0-9a-f]{32}$/.test(data.salt ?? "")) throw new Error("Invalid app-PIN derivation request.");
        // The prefix provides domain separation, not additional PIN entropy.
        // The result is used only for a device-key-wrapped inner envelope.
        return wasm.derive_vault_key(`LocalWalletAppPIN/v1/${data.password}`, fromHex(data.salt!));
      }
      case "create":
      case "restore": {
        const password = data.password ?? "";
        if (password.length < 12 || password.length > 1024) throw new Error("Use a password of 12 to 1,024 characters.");
        const salt = crypto.getRandomValues(new Uint8Array(16));
        key = wasm.derive_vault_key(password, salt);
        const generated = JSON.parse(operation === "create" ? wasm.generate_wallet(key) : wasm.restore_wallet(data.mnemonic ?? "", key));
        const vault: Vault = {
          id: "primary", version: 1, kdf: "argon2id-64m-t3-p1",
          salt: Array.from(salt, byte => byte.toString(16).padStart(2, "0")).join(""),
          vault_hex: generated.vault_hex, address: generated.address,
          created_at: Math.floor(Date.now() / 1000), backup_confirmed: operation === "restore",
        };
        validateVault(vault);
        return operation === "create" ? { vault, mnemonic: generated.mnemonic } : vault;
      }
      case "authenticate":
      case "reveal":
      case "sign": {
        const vault = validateVault(data.vault);
        key = keyFor(vault, data.password ?? "");
        const authenticatedAddress = verifyAddress(vault, key);
        if (operation === "authenticate") return authenticatedAddress;
        if (operation === "reveal") return wasm.reveal_mnemonic(vault.vault_hex, key);
        const prepared = wasm.validate_prepared_json(JSON.stringify(data.prepared));
        return JSON.parse(wasm.sign_payment(vault.vault_hex, key, prepared, JSON.parse(prepared).approval_digest));
      }
      case "prepare": return JSON.parse(wasm.prepare_payment(JSON.stringify(data.request)));
      case "validatePrepared": return JSON.parse(wasm.validate_prepared_json(JSON.stringify(data.prepared)));
      case "validateSigned": return JSON.parse(wasm.validate_signed_for_prepared(JSON.stringify(data.prepared), JSON.stringify(data.signed)));
      case "validateReceipt":
        if (!Number.isSafeInteger(data.finalizedHeight) || data.finalizedHeight! < 0) throw new Error("Invalid finality height.");
        return JSON.parse(wasm.validate_receipt(JSON.stringify(data.signed), JSON.stringify(data.receipt), data.canonicalHash ?? "", BigInt(data.finalizedHeight!)));
      case "validateActivity": {
        const proof=data.proof;
        if(!proof || !Number.isSafeInteger(proof.finalized_height) || proof.finalized_height<0 || ![1,8453].includes(data.chain ?? 0)) throw new Error("Invalid activity verification request.");
        return JSON.parse(wasm.validate_activity(BigInt(data.chain!),data.address ?? "",JSON.stringify(proof.transaction),JSON.stringify(proof.receipt),JSON.stringify(proof.canonical_block),BigInt(proof.finalized_height)));
      }
      default: throw new Error("Unsupported wallet operation.");
    }
  } finally {
    key?.fill(0);
    data.password = "";
    data.mnemonic = "";
  }
}
self.onmessage = async (event: MessageEvent<{ operation: string; data: Payload }>) => {
  const { operation, data } = event.data;
  let value: unknown;
  try {
    await init();
    value = run(operation, data);
    self.postMessage({ ok: true, value });
  } catch (error) {
    // Do not reflect user secret input or implementation stack traces to UI/logs.
    const detail = secretOperations.has(operation)
      ? (operation === "restore" ? "Restore failed. Check the recovery words and password requirements." : "Wallet operation failed. Check your password, review expiry, and wallet data.")
      : String(error instanceof Error ? error.message : error).slice(0, 360);
    self.postMessage({ ok: false, error: detail });
  } finally {
    if (value instanceof Uint8Array) value.fill(0);
    self.close();
  }
};
