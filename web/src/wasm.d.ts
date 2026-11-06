declare module "*crypto/wallet_core.js" {
  export default function init(input?: unknown): Promise<unknown>;
  export function generate_wallet(key: Uint8Array): string;
  export function restore_wallet(mnemonic: string, key: Uint8Array): string;
  export function reveal_mnemonic(vault: string, key: Uint8Array): string;
  export function wallet_address(vault: string, key: Uint8Array): string;
  export function prepare_payment(request: string): string;
  export function sign_payment(vault: string, key: Uint8Array, prepared: string, approval: string): string;
  export function inspect_signed(raw: string): string;
  export function asset_registry(): string;
  export function derive_vault_key(password: string, salt: Uint8Array): Uint8Array;
  export function validate_prepared_json(prepared: string): string;
  export function validate_signed_for_prepared(prepared: string, signed: string): string;
  export function validate_receipt(signed: string, receipt: string, hash: string, finalized: bigint): string;
  export function validate_activity(chain: bigint, address: string, transaction: string, receipt: string, block: string, finalized: bigint): string;
}
