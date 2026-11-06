export type Asset = "ETH" | "USDC" | "EURC";
export type ChainId = 1 | 8453;
export interface NetworkAsset { symbol: Asset; decimals: number; contract: string | null }
export interface Network {
  chain_id: ChainId;
  network: string;
  rpc_url: string;
  broadcast_url: string;
  explorer_url: string;
  assets: NetworkAsset[];
}
export interface Vault {
  id: "primary";
  version: 1;
  kdf: "argon2id-64m-t3-p1";
  salt: string;
  vault_hex: string;
  address: string;
  created_at: number;
  backup_confirmed: boolean;
  /** Present only when a matching device-protection record was committed. */
  protection?: "device-key-v1";
}
export interface VaultProtectionRecord {
  id: "primary";
  version: 1;
  mode: "off" | "pin";
  revision: number;
  key_id: string;
  vault_binding: string;
  device_key: CryptoKey;
  iv: string;
  ciphertext: string;
  pin_salt: string | null;
  failed_attempts: number;
  retry_at: number;
  attempt_serial: number;
}
export interface VaultProtectionState { vault: Vault; protection: VaultProtectionRecord | null }
export interface VaultProtectionAttempt { revision: number; key_id: string; serial: number }
export interface PaymentRequest {
  intent_id: string;
  chain_id: ChainId;
  from: string;
  to: string;
  asset: Asset;
  amount: string;
  nonce: number;
  gas_limit: number;
  max_fee_per_gas: string;
  max_priority_fee_per_gas: string;
  l1_fee_estimate_wei: string;
  operator_fee_estimate_wei: string;
}
export interface PreparedPayment {
  request: PaymentRequest;
  amount_units: string;
  max_fee_wei: string;
  estimated_total_fee_wei: string;
  fee_reserve_wei: string;
  unsigned_transaction: string;
  approval_digest: string;
  created_at: number;
  expires_at: number;
}
export interface SignedPayment {
  intent_id: string;
  chain_id: ChainId;
  from: string;
  to: string;
  asset: Asset;
  amount: string;
  amount_units: string;
  nonce: number;
  max_fee_wei: string;
  l1_fee_estimate_wei: string;
  operator_fee_estimate_wei: string;
  estimated_total_fee_wei: string;
  fee_reserve_wei: string;
  approval_digest: string;
  raw_transaction: string;
  hash: string;
}
export type Phase = "prepared" | "signed" | "unknown" | "pending" | "included" | "finalized" | "unresolved" | "cancelled";
export type Execution = "unknown" | "success" | "failed";
export type Inclusion = "unseen" | "included" | "finalized";
export interface ReceiptValidation {
  phase: "included" | "finalized" | "unresolved";
  execution: Execution;
  inclusion: "included" | "finalized";
  block_number: number;
  block_hash: string;
  receipt_status: number;
  receipt_fingerprint: string;
  actual_fee_wei: string;
  actual_execution_fee_wei: string;
  actual_l1_fee_wei: string;
  actual_operator_fee_wei: string;
  additional_fee_exceeded_estimate: boolean;
  resolution_note: string | null;
  evidence: unknown;
}
export interface PaymentRecord {
  intent_id: string;
  account: string;
  nonce_key?: string;
  prepared: PreparedPayment;
  signed: SignedPayment | null;
  phase: Phase;
  execution: Execution;
  inclusion: Inclusion;
  receipt: ReceiptValidation | null;
  receipt_proof: { receipt: unknown; canonical_hash: string; finalized_height: number } | null;
  resolution_note: string | null;
  created_at: number;
  updated_at: number;
  revision: number;
}
export interface JournalEvent {
  id?: number;
  intent_id: string;
  time: number;
  kind: string;
  evidence: unknown;
}
export interface BalanceSnapshot {
  chain_id: ChainId;
  address: string;
  block_number: number;
  block_hash: string;
  balances: Record<Asset, string>;
  observed_at: number;
}
export interface ConnectionInfo {
  chain_id: ChainId;
  provider: string | null;
  endpoint: string | null;
  transport: "relay" | "direct" | null;
  checked_at: number | null;
  failed_attempts: number;
}
export interface BalanceStatus {
  status: "fresh" | "unavailable";
  snapshot: BalanceSnapshot | null;
  provider: string | null;
  observed_at: number;
  error: string | null;
}
export interface FeeDraft { from: string; to: string; asset: Asset; amount: string }
export interface FeePreview {
  chain_id: ChainId;
  asset: Asset;
  estimated_execution_fee_wei: string;
  l1_fee_estimate_wei: string;
  operator_fee_estimate_wei: string;
  estimated_total_fee_wei: string;
  max_execution_fee_wei: string;
  fee_reserve_wei: string;
  gas_limit: number;
  max_fee_per_gas: string;
  max_priority_fee_per_gas: string;
  provider: string | null;
  estimated_at: number;
  estimate_source: "simulation" | "conservative";
  warnings: string[];
  shortages: string[];
}
export interface ChainActivity {
  id: string;
  chain_id: ChainId;
  address: string;
  direction: "received" | "sent" | "self";
  asset: Asset;
  asset_contract: string | null;
  amount_units: string;
  amount: string;
  from: string;
  to: string;
  transaction_hash: string;
  block_number: number;
  block_hash: string;
  log_index: number | null;
  timestamp: number;
  inclusion: "included" | "finalized" | "reorged";
}
export interface ActivityProof {
  transaction: unknown;
  receipt: unknown;
  canonical_block: unknown;
  finalized_height: number;
}
export interface ActivityValidation { activities: ChainActivity[] }
export interface ActivityResult {
  items: ChainActivity[];
  coverage: "recent" | "lookup" | "partial" | "unavailable";
  complete: false;
  from_block: number | null;
  to_block: number | null;
  provider: string | null;
  observed_at: number;
  warnings: string[];
  error: string | null;
  lookup_status?: "received" | "sent" | "self" | "pending" | "not_found" | "unrelated" | "failed" | "unavailable";
}
export interface Block {
  number: string;
  hash: string;
  baseFeePerGas: string;
  timestamp: string;
}
export interface CoreApi {
  registry(): Promise<{ networks: Network[] }>;
  create(password: string): Promise<{ vault: Vault; mnemonic: string }>;
  restore(password: string, mnemonic: string): Promise<Vault>;
  authenticate(vault: Vault, password: string): Promise<string>;
  reveal(vault: Vault, password: string): Promise<string>;
  /** Worker-bound Argon2id; six-digit app PIN is domain separated, never used as a core vault password. */
  deriveProtectionKey?(pin: string, salt: string): Promise<Uint8Array>;
  prepare(request: PaymentRequest): Promise<PreparedPayment>;
  sign(vault: Vault, password: string, prepared: PreparedPayment): Promise<SignedPayment>;
  validatePrepared(prepared: PreparedPayment): Promise<PreparedPayment>;
  validateSigned(prepared: PreparedPayment, signed: SignedPayment): Promise<SignedPayment>;
  validateReceipt(signed: SignedPayment, receipt: unknown, canonicalHash: string, finalizedHeight: number): Promise<ReceiptValidation>;
  validateActivity?(chain: ChainId, address: string, proof: ActivityProof): Promise<ActivityValidation>;
}
