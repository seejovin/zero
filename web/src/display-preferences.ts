/** Cosmetic, device-local labels. Never use these values as wallet or signing identity. */
export interface DisplayNames { walletName: string; accountName: string }
export interface DisplayStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export const DISPLAY_NAME_MAX_LENGTH = 32;
export const DEFAULT_DISPLAY_NAMES: Readonly<DisplayNames> = Object.freeze({ walletName: "Wallet 1", accountName: "Account 1" });
const KEY_PREFIX = "local-wallet-display-v1:";
const unsupportedCharacters = /[\p{Cc}\p{Cf}\p{Cs}\p{Zl}\p{Zp}]/u;

/** Names are Unicode text, not markup. Callers must render them through text nodes. */
export function normalizeDisplayName(value: string): string {
  if (typeof value !== "string" || value.length > 256 || unsupportedCharacters.test(value)) {
    throw new Error("Names cannot contain control characters, line breaks or hidden formatting.");
  }
  const name = value.normalize("NFC").trim();
  if (name.length === 0 || Array.from(name).length > DISPLAY_NAME_MAX_LENGTH) {
    throw new Error(`Use a name with 1–${DISPLAY_NAME_MAX_LENGTH} characters.`);
  }
  return name;
}

function identity(value: string): string {
  if (typeof value !== "string" || value.length !== 42 || !/^0x[0-9a-fA-F]{40}$/.test(value) || /^0x0{40}$/i.test(value)) {
    throw new Error("The wallet address is invalid.");
  }
  return value.toLowerCase();
}

function normalized(names: DisplayNames): DisplayNames {
  return { walletName: normalizeDisplayName(names?.walletName), accountName: normalizeDisplayName(names?.accountName) };
}

/** Unavailable or invalid cosmetic preferences must never block opening the wallet. */
export function readDisplayNames(address: string, storage?: DisplayStorage): DisplayNames {
  const account = identity(address);
  try {
    const raw = (storage ?? globalThis.localStorage).getItem(KEY_PREFIX + account);
    if (raw === null || raw.length > 1_024) return { ...DEFAULT_DISPLAY_NAMES };
    const record: unknown = JSON.parse(raw);
    if (!record || typeof record !== "object" || Array.isArray(record)) return { ...DEFAULT_DISPLAY_NAMES };
    const saved = record as DisplayNames & { version?: unknown; address?: unknown };
    if (saved.version !== 1 || saved.address !== account) return { ...DEFAULT_DISPLAY_NAMES };
    return normalized(saved);
  } catch {
    return { ...DEFAULT_DISPLAY_NAMES };
  }
}

/** One record keeps the two names together. A save never opens or modifies the encrypted vault. */
export function saveDisplayNames(address: string, names: DisplayNames, storage?: DisplayStorage): DisplayNames {
  const account = identity(address);
  const checked = normalized(names);
  const serialized = JSON.stringify({ version: 1, address: account, ...checked });
  try {
    const target = storage ?? globalThis.localStorage;
    const key = KEY_PREFIX + account;
    target.setItem(key, serialized);
    if (target.getItem(key) !== serialized) throw new Error("Display-name readback did not match.");
  } catch {
    // A write might have completed before verification failed. Do not claim it was rolled back.
    throw new Error("Your display names could not be confirmed. Reopen Profile to check them, then try again.");
  }
  return checked;
}
