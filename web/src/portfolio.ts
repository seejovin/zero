import type { Asset, ChainId } from "./types";

/** A position in the fixed, issuer-verified asset registry; a symbol alone is not a selection. */
export interface AssetRef { chainId: ChainId; symbol: Asset }
export interface ChainUsdcBalance { chainId: ChainId; amount: string | null }

export function validateAssetRef(value: unknown): AssetRef {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Select a supported asset and network.");
  const ref = value as Partial<AssetRef>;
  if ((ref.chainId !== 1 && ref.chainId !== 8453) || (ref.symbol !== "USDC" && ref.symbol !== "EURC" && ref.symbol !== "ETH")) {
    throw new Error("Select a supported asset and network.");
  }
  return { chainId: ref.chainId, symbol: ref.symbol };
}

export function assetRefKey(value: AssetRef): string {
  const ref = validateAssetRef(value);
  return `${ref.chainId}:${ref.symbol}`;
}

const SCALE = 1_000_000n;
const MAX_CHAIN_UNITS = (1n << 256n) - 1n;
function usdcUnits(amount: unknown): bigint | null {
  if (typeof amount !== "string" || amount.length > 90 || amount !== amount.trim() || !/^(0|[1-9][0-9]*)(\.[0-9]{1,6})?$/.test(amount)) return null;
  const [whole, fraction = ""] = amount.split(".");
  const units = BigInt(whole) * SCALE + BigInt(fraction.padEnd(6, "0"));
  return units <= MAX_CHAIN_UNITS ? units : null;
}

/**
 * Exact USDC quantity across Base and Ethereum, not a fiat valuation or a spendable balance.
 * Both networks are required. Callers retain each observation's age and stale/partial status.
 */
export function totalUsdc(entries: readonly ChainUsdcBalance[]): string | null {
  if (!Array.isArray(entries) || entries.length !== 2) return null;
  const seen = new Set<ChainId>();
  let total = 0n;
  for (const entry of entries) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry) || (entry.chainId !== 1 && entry.chainId !== 8453) || seen.has(entry.chainId)) return null;
    const amount = usdcUnits(entry.amount);
    if (amount === null) return null;
    seen.add(entry.chainId);
    total += amount;
  }
  const fraction = (total % SCALE).toString().padStart(6, "0").replace(/0+$/, "");
  return `${total / SCALE}${fraction ? `.${fraction}` : ""}`;
}
