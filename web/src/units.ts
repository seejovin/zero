import type { Asset, ChainId, Network, PreparedPayment, Vault } from "./types";

const U256_MAX = (1n << 256n) - 1n;
export function units(value: string, decimals: number): bigint {
  if (!Number.isInteger(decimals) || decimals < 0 || decimals > 18 || value.length > 100 || !/^(0|[1-9][0-9]*)(\.[0-9]+)?$/.test(value)) {
    throw new Error("Enter an exact positive amount using digits and a decimal point.");
  }
  const [whole, fraction = ""] = value.split(".");
  if (fraction.length > decimals) throw new Error(`This asset supports at most ${decimals} decimal places.`);
  const result = BigInt(whole) * 10n ** BigInt(decimals) + BigInt((fraction + "0".repeat(decimals)).slice(0, decimals) || "0");
  if (result > U256_MAX) throw new Error("Amount exceeds the supported range.");
  return result;
}
export function formatUnits(value: string | bigint, decimals: number): string {
  const number = typeof value === "string" ? decimal(value) : value;
  if (number < 0n || !Number.isInteger(decimals) || decimals < 0 || decimals > 18) throw new Error("Invalid amount.");
  const factor = 10n ** BigInt(decimals);
  if (!decimals) return number.toString();
  const tail = (number % factor).toString().padStart(decimals, "0").replace(/0+$/, "");
  return `${number / factor}${tail ? `.${tail}` : ""}`;
}
export function decimal(value: unknown): bigint {
  if (typeof value !== "string" || value.length > 78 || !/^(0|[1-9][0-9]*)$/.test(value)) throw new Error("Invalid decimal quantity.");
  const result = BigInt(value);
  if (result > U256_MAX) throw new Error("Quantity exceeds 256 bits.");
  return result;
}
export function quantity(value: unknown): bigint {
  if (typeof value !== "string" || !/^0x(0|[1-9a-fA-F][0-9a-fA-F]{0,63})$/.test(value)) throw new Error("The network returned an invalid quantity.");
  return BigInt(value);
}
export function safeNumber(value: bigint): number {
  if (value < 0n || value > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error("The network returned a number outside the supported range.");
  return Number(value);
}
export function hexQuantity(value: bigint | number): string {
  const v = BigInt(value);
  if (v < 0n || v > U256_MAX) throw new Error("Invalid hexadecimal quantity.");
  return `0x${v.toString(16)}`;
}
export function abiUint(value: unknown): bigint {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(value)) throw new Error("The network returned an invalid contract result.");
  return BigInt(value);
}
export function hash(value: unknown): string {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(value)) throw new Error("The network returned an invalid block or transaction hash.");
  return value.toLowerCase();
}
export function address(value: unknown): string {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{40}$/.test(value) || /^0x0{40}$/i.test(value)) throw new Error("Enter a valid 0x address. Names and arbitrary payment links are not supported.");
  return value;
}
export function accountKey(chain: ChainId, from: string): string {
  if (chain !== 1 && chain !== 8453) throw new Error("Unsupported network.");
  return `${chain}:${address(from).toLowerCase()}`;
}
export function networkAsset(network: Network, symbol: Asset) {
  const found = network.assets.find(a => a.symbol === symbol);
  if (!found) throw new Error("Unsupported asset.");
  return found;
}
export function sameAddress(a: string, b: string): boolean { return a.toLowerCase() === b.toLowerCase(); }
export function shortAddress(value: string): string { return `${value.slice(0, 8)}…${value.slice(-6)}`; }
export function constantJson(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(constantJson).join(",")}]`;
  return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${constantJson((value as Record<string, unknown>)[k])}`).join(",")}}`;
}
export function validateVault(value: unknown): Vault {
  const v = value as Vault;
  if (!v || v.id !== "primary" || v.version !== 1 || v.kdf !== "argon2id-64m-t3-p1" || !/^[0-9a-f]{32}$/.test(v.salt) || !/^0x[0-9a-f]+$/i.test(v.vault_hex) || v.vault_hex.length > 4096 || typeof v.backup_confirmed !== "boolean" || !Number.isSafeInteger(v.created_at)) throw new Error("The local wallet record is invalid. Do not send until it has been investigated.");
  if (v.protection !== undefined && v.protection !== "device-key-v1") throw new Error("This wallet protection format is not supported. Existing wallet data has been preserved.");
  address(v.address);
  return v;
}
export function validatePreparedIdentity(prepared: PreparedPayment, vault: Vault, chain: ChainId): void {
  if (prepared.request.chain_id !== chain || !sameAddress(prepared.request.from, vault.address)) throw new Error("Payment belongs to a different wallet or network.");
}
export function receiveUri(network: Network, from: string, asset: Asset): string {
  address(from);
  const spec = networkAsset(network, asset);
  return spec.contract ? `ethereum:${spec.contract}@${network.chain_id}/transfer?address=${from}` : `ethereum:${from}@${network.chain_id}`;
}
