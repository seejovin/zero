import { test } from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_DISPLAY_NAMES, normalizeDisplayName, readDisplayNames, saveDisplayNames, type DisplayStorage } from "../src/display-preferences";

const address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const other = "0x1111111111111111111111111111111111111111";
class MemoryStorage implements DisplayStorage {
  readonly values = new Map<string, string>();
  getItem(key: string): string | null { return this.values.get(key) ?? null; }
  setItem(key: string, value: string): void { this.values.set(key, value); }
}

test("existing wallets receive defaults without a migration; renamed labels survive a new reader and address casing", () => {
  const storage = new MemoryStorage();
  assert.deepEqual(readDisplayNames(address, storage), DEFAULT_DISPLAY_NAMES);
  assert.equal(storage.values.size, 0);
  const names = saveDisplayNames(address, { walletName: "  Savings  ", accountName: "日常账户" }, storage);
  assert.deepEqual(names, { walletName: "Savings", accountName: "日常账户" });
  const reopened: DisplayStorage = { getItem: key => storage.getItem(key), setItem: (key, value) => storage.setItem(key, value) };
  assert.deepEqual(readDisplayNames(address.toLowerCase(), reopened), names);
  assert.deepEqual(readDisplayNames(other, reopened), DEFAULT_DISPLAY_NAMES);
  assert.equal(storage.values.size, 1);
  assert.deepEqual(JSON.parse([...storage.values.values()][0]), { version: 1, address: address.toLowerCase(), ...names });
});

test("renaming is scoped to cosmetic keys and does not replace vault, protection or unrelated account data", () => {
  const storage = new MemoryStorage();
  storage.setItem("encrypted-vault-fixture", "unchanged public sentinel");
  storage.setItem("protection-fixture", "unchanged policy sentinel");
  const otherNames = saveDisplayNames(other, { walletName: "Other wallet", accountName: "Other account" }, storage);
  saveDisplayNames(address, { walletName: "Travel", accountName: "Spending" }, storage);
  saveDisplayNames(address, { walletName: "Emergency savings", accountName: "Reserve" }, storage);
  assert.equal(storage.getItem("encrypted-vault-fixture"), "unchanged public sentinel");
  assert.equal(storage.getItem("protection-fixture"), "unchanged policy sentinel");
  assert.deepEqual(readDisplayNames(other, storage), otherNames);
  assert.equal(storage.values.size, 4);
});

test("invalid or deceptive labels are rejected before modifying the saved names", () => {
  const storage = new MemoryStorage();
  const saved = saveDisplayNames(address, { walletName: "Savings", accountName: "Reserve" }, storage);
  const previous = [...storage.values.values()][0];
  for (const invalid of ["", "   ", "x".repeat(33), "line\nbreak", "null\u0000byte", "hidden\u200bname", "reverse\u202ename", "isolate\u2066name", "line\u2028break", "\ud800", "\udfff", " ".repeat(257)]) {
    assert.throws(() => saveDisplayNames(address, { walletName: invalid, accountName: "Changed" }, storage));
    assert.equal([...storage.values.values()][0], previous);
  }
  assert.deepEqual(readDisplayNames(address, storage), saved);
  assert.equal(normalizeDisplayName("Cafe\u0301"), "Café");
  assert.equal(normalizeDisplayName("🔒".repeat(32)), "🔒".repeat(32));
  assert.throws(() => normalizeDisplayName("🔒".repeat(33)));
  // Markup-looking input remains literal text; the rendering boundary must use text nodes.
  assert.equal(normalizeDisplayName("<b>Savings & cash</b>"), "<b>Savings & cash</b>");
});

test("malformed, unsupported, oversized and wrong-account preferences fall back without repairing or overwriting them", () => {
  const storage = new MemoryStorage();
  saveDisplayNames(address, { walletName: "Original", accountName: "Original" }, storage);
  const key = [...storage.values.keys()][0];
  for (const raw of ["{", "null", "[]", "x".repeat(1_025),
    JSON.stringify({ version: 2, address: address.toLowerCase(), walletName: "Wrong", accountName: "Wrong" }),
    JSON.stringify({ version: 1, address: other, walletName: "Wrong", accountName: "Wrong" }),
    JSON.stringify({ version: 1, address: address.toLowerCase(), walletName: "Spoof\u202e", accountName: "Wrong" }),
    JSON.stringify({ version: 1, address: address.toLowerCase(), walletName: 42, accountName: "Wrong" })]) {
    storage.setItem(key, raw);
    assert.deepEqual(readDisplayNames(address, storage), DEFAULT_DISPLAY_NAMES);
    assert.equal(storage.getItem(key), raw);
  }
  const unavailable: DisplayStorage = { getItem() { throw new Error("private storage detail"); }, setItem() { throw new Error("private storage detail"); } };
  assert.deepEqual(readDisplayNames(address, unavailable), DEFAULT_DISPLAY_NAMES);
});

test("failed writes and failed readback never report a successful rename or leak storage errors", () => {
  const names = { walletName: "Savings", accountName: "Reserve" };
  const failure: DisplayStorage = { getItem() { return null; }, setItem() { throw new Error("private/path quota details"); } };
  const mismatch: DisplayStorage = { getItem() { return null; }, setItem() {} };
  for (const storage of [failure, mismatch]) {
    assert.throws(() => saveDisplayNames(address, names, storage), error => {
      assert.equal((error as Error).message, "Your display names could not be confirmed. Reopen Profile to check them, then try again.");
      return true;
    });
  }
  const saved = new MemoryStorage();
  saveDisplayNames(address, names, saved);
  const refused: DisplayStorage = { getItem: key => saved.getItem(key), setItem() { throw new Error("quota"); } };
  assert.throws(() => saveDisplayNames(address, { walletName: "Replacement", accountName: "Replacement" }, refused));
  assert.deepEqual(readDisplayNames(address, saved), names);
  for (const invalid of ["", "not-an-address", "0x" + "0".repeat(40), address + "x", address + "\n"]) {
    assert.throws(() => saveDisplayNames(invalid, names, saved));
  }
  assert.equal(saved.values.size, 1);
});
