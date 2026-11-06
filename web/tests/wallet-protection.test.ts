import { test } from "node:test";
import assert from "node:assert/strict";
import { IDBFactory } from "fake-indexeddb";
import { Journal } from "../src/journal";
import { WalletProtection, validateAppPin } from "../src/wallet-protection";
import { fixtureCore, wasm, PUBLIC_FIXTURE, FROM, prepare } from "./fixtures";
import type { CoreApi, Vault, VaultProtectionRecord } from "../src/types";

// Public, unfunded development fixtures only; no network or application profile.
const PASSWORD = "public legacy password only";
const PIN = "648271";
const OTHER_PIN = "830592";
const WRONG_PIN = "999999";
function keyFor(vault: Vault, password: string): Uint8Array {
  return wasm.derive_vault_key(password, Uint8Array.from(vault.salt.match(/../g)!, v => parseInt(v, 16)));
}
function coreVault(password: string, words: string | null): { vault: Vault; mnemonic: string } {
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const key = wasm.derive_vault_key(password, salt);
  try {
    const value = JSON.parse(words === null ? wasm.generate_wallet(key) : wasm.restore_wallet(words, key));
    return { vault: { id: "primary", version: 1, kdf: "argon2id-64m-t3-p1", salt: Buffer.from(salt).toString("hex"),
      vault_hex: value.vault_hex, address: value.address, created_at: 1_700_000_000, backup_confirmed: words !== null }, mnemonic: words ?? value.mnemonic };
  } finally { key.fill(0); }
}
const actualCore: CoreApi = {
  ...fixtureCore,
  create: async password => coreVault(password, null),
  restore: async (password, words) => coreVault(password, words).vault,
  authenticate: async (vault, password) => {
    const key = keyFor(vault, password);
    try { return wasm.wallet_address(vault.vault_hex, key); } finally { key.fill(0); }
  },
  reveal: async (vault, password) => {
    const key = keyFor(vault, password);
    try { return wasm.reveal_mnemonic(vault.vault_hex, key); } finally { key.fill(0); }
  },
  deriveProtectionKey: async (pin, salt) => wasm.derive_vault_key(`LocalWalletAppPIN/v1/${pin}`, Uint8Array.from(salt.match(/../g)!, v => parseInt(v, 16))),
};
async function empty(core: CoreApi = actualCore, clock: () => number = Date.now) {
  const factory = new IDBFactory(), name = crypto.randomUUID();
  const journal = await Journal.open(core, factory, name);
  return { factory, name, journal, protection: new WalletProtection(core, journal, clock) };
}
function database(journal: Journal): IDBDatabase { return (journal as unknown as { db: IDBDatabase }).db; }
async function raw(journal: Journal, store: string): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const r = database(journal).transaction(store).objectStore(store).get("primary");
    r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
  });
}
async function mutate(journal: Journal, store: string, operation: (objectStore: IDBObjectStore) => void): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const tx = database(journal).transaction(store, "readwrite");
    operation(tx.objectStore(store)); tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
  });
}
function abortProtectionWrite(journal: Journal): () => void {
  const db = database(journal), original = db.transaction.bind(db);
  db.transaction = ((stores: string | string[], mode?: IDBTransactionMode, options?: IDBTransactionOptions) => {
    const tx = original(stores, mode, options);
    if (mode === "readwrite" && Array.from(typeof stores === "string" ? [stores] : stores).includes("protection")) {
      const originalStore = tx.objectStore.bind(tx);
      tx.objectStore = ((name: string) => {
        const store = originalStore(name);
        if (name === "protection") for (const method of ["put", "add"] as const) {
          const originalWrite = store[method].bind(store);
          store[method] = ((value: unknown, key?: IDBValidKey) => {
            const r = originalWrite(value, key); r.addEventListener("success", () => tx.abort(), { once: true }); return r;
          }) as typeof store.put;
        }
        return store;
      }) as typeof tx.objectStore;
    }
    return tx;
  }) as typeof db.transaction;
  return () => { db.transaction = original; };
}

test("managed creation resumes the identical 24 words after quitting before backup confirmation", async () => {
  const { journal, protection, factory, name } = await empty();
  const created = await protection.create();
  assert.equal(created.mnemonic.split(" ").length, 24);
  assert.equal(created.vault.backup_confirmed, false);
  assert.equal((await protection.status(created.vault)).mode, "off");
  const stored = await raw(journal, "protection") as VaultProtectionRecord;
  assert.equal(stored.device_key.extractable, false);
  await assert.rejects(crypto.subtle.exportKey("raw", stored.device_key));
  const originalSecret = await protection.credentials(created.vault);
  assert.match(originalSecret, /^[0-9a-f]{64}$/);
  assert.ok(!JSON.stringify(stored).includes(created.mnemonic));
  assert.ok(!JSON.stringify(stored).includes(originalSecret));
  journal.close();
  const reopened = await Journal.open(actualCore, factory, name);
  const resumed = (await reopened.vault())!;
  const service = new WalletProtection(actualCore, reopened);
  assert.equal(resumed.address, created.vault.address);
  assert.equal(resumed.vault_hex, created.vault.vault_hex);
  assert.equal(resumed.backup_confirmed, false);
  const secret = await service.credentials(resumed);
  assert.equal(secret, originalSecret);
  assert.equal(await actualCore.reveal(resumed, secret), created.mnemonic);
  const confirmed = await reopened.confirmBackup(resumed);
  assert.equal(confirmed.backup_confirmed, true);
  assert.equal(await service.credentials(confirmed), originalSecret);
});

test("managed restore needs no user passcode and preserves the compatible account", async () => {
  const { journal, protection } = await empty();
  const vault = await protection.restore(PUBLIC_FIXTURE);
  assert.equal(vault.address, FROM); assert.equal(vault.backup_confirmed, true);
  assert.deepEqual(await protection.status(vault), { mode: "off", retry_at: 0, failed_attempts: 0 });
  assert.equal(await actualCore.reveal(vault, await protection.credentials(vault)), PUBLIC_FIXTURE);
  await assert.rejects(protection.restore(PUBLIC_FIXTURE), /already exists/);
  assert.deepEqual(await journal.vault(), vault);
});

test("PIN length is exactly six digits, and PIN protection is cryptographic rather than a UI flag", async () => {
  for (const pin of ["", "1234", "1234567", "12a456", "123456 ", "１２３４５６"]) assert.throws(() => validateAppPin(pin), /exactly 6/);
  validateAppPin("000001");
  const { journal, protection } = await empty();
  const vault = await protection.restore(PUBLIC_FIXTURE);
  const secret = await protection.credentials(vault);
  const pinVault = await protection.setLock(vault, "", PIN);
  assert.equal((await protection.status(pinVault)).mode, "pin");
  assert.equal(pinVault.vault_hex, vault.vault_hex);
  await assert.rejects(protection.credentials(pinVault), /exactly 6/);
  await assert.rejects(protection.credentials(pinVault, WRONG_PIN), /Incorrect 6-digit/);
  assert.equal(await protection.credentials(pinVault, PIN), secret);
  // Editing only the persisted mode cannot bypass the authenticated encryption.
  const stored = await raw(journal, "protection") as VaultProtectionRecord;
  await mutate(journal, "protection", store => store.put({ ...stored, mode: "off", pin_salt: null, failed_attempts: 0, retry_at: 0, attempt_serial: 0 }));
  await assert.rejects(protection.credentials(pinVault), /device protection/);
});

test("PIN change and removal require current authentication and replace their device envelope", async () => {
  const { journal, protection } = await empty();
  let vault = await protection.restore(PUBLIC_FIXTURE);
  const coreSecret = await protection.credentials(vault);
  vault = await protection.setLock(vault, "", PIN);
  const first = await raw(journal, "protection") as VaultProtectionRecord;
  await assert.rejects(protection.setLock(vault, WRONG_PIN, null), /Incorrect 6-digit/);
  assert.equal((await protection.status(vault)).mode, "pin");
  vault = await protection.setLock(vault, PIN, OTHER_PIN);
  await assert.rejects(protection.credentials(vault, PIN), /Incorrect 6-digit/);
  assert.equal(await protection.credentials(vault, OTHER_PIN), coreSecret);
  vault = await protection.setLock(vault, OTHER_PIN, null);
  assert.equal((await protection.status(vault)).mode, "off");
  assert.equal(await protection.credentials(vault), coreSecret);
  const last = await raw(journal, "protection") as VaultProtectionRecord;
  assert.notEqual(first.key_id, last.key_id);
  assert.equal(last.revision, 4);
});

test("incorrect PIN reservations and backoff survive reload without an unlock bypass", async () => {
  let clock = 1_800_000_000_000;
  const { journal, protection, factory, name } = await empty(actualCore, () => clock);
  const vault = await protection.setLock(await protection.restore(PUBLIC_FIXTURE), "", PIN);
  for (let i = 0; i < 5; i++) await assert.rejects(protection.credentials(vault, WRONG_PIN), /Incorrect 6-digit/);
  const state = await protection.status(vault);
  assert.equal(state.failed_attempts, 5); assert.equal(state.retry_at, clock + 30_000);
  journal.close();
  const reopened = await Journal.open(actualCore, factory, name);
  const service = new WalletProtection(actualCore, reopened, () => clock);
  await assert.rejects(service.credentials(vault, PIN), /Try again in 30 seconds/);
  clock += 30_001;
  assert.equal(await actualCore.reveal(vault, await service.credentials(vault, PIN)), PUBLIC_FIXTURE);
  assert.deepEqual(await service.status(vault), { mode: "pin", retry_at: 0, failed_attempts: 0 });
});

test("concurrent PIN guesses cannot exceed the durable reservation threshold", async () => {
  const { protection } = await empty(actualCore, () => 1_800_000_000_000);
  const vault = await protection.setLock(await protection.restore(PUBLIC_FIXTURE), "", PIN);
  const results = await Promise.allSettled(Array.from({ length: 12 }, () => protection.credentials(vault, WRONG_PIN)));
  assert.equal(results.every(result => result.status === "rejected"), true);
  assert.equal((await protection.status(vault)).failed_attempts, 5);
  assert.equal(results.filter(result => result.status === "rejected" && /Too many/.test(String(result.reason))).length, 7);
});

test("failed initial storage commit cannot leave a vault without its wrapping key", async () => {
  const { journal, protection, factory, name } = await empty();
  const restoreWrites = abortProtectionWrite(journal);
  try { await assert.rejects(protection.create(), /commit|abort/i); } finally { restoreWrites(); }
  assert.equal(await journal.vault(), null);
  assert.equal(await raw(journal, "protection"), undefined);
  journal.close();
  const reopened = await Journal.open(actualCore, factory, name);
  assert.equal(await reopened.vault(), null);
});

test("failed lock-change commit leaves the prior vault and device envelope usable", async () => {
  const { journal, protection } = await empty();
  const vault = await protection.restore(PUBLIC_FIXTURE);
  const before = await raw(journal, "protection") as VaultProtectionRecord;
  const restoreWrites = abortProtectionWrite(journal);
  try { await assert.rejects(protection.setLock(vault, "", PIN), /commit|abort/i); } finally { restoreWrites(); }
  const after = await raw(journal, "protection") as VaultProtectionRecord;
  assert.equal(after.key_id, before.key_id); assert.equal(after.ciphertext, before.ciphertext);
  assert.deepEqual(await journal.vault(), vault);
  assert.equal((await protection.status(vault)).mode, "off");
  assert.equal(await actualCore.reveal(vault, await protection.credentials(vault)), PUBLIC_FIXTURE);
});

test("legacy password protection is retained until explicit migration, which rekeys without changing the account or history", async () => {
  const { journal, protection } = await empty();
  const legacy = coreVault(PASSWORD, PUBLIC_FIXTURE).vault;
  await journal.saveVault(legacy);
  await journal.recordPrepared(prepare("preserve-draft"));
  const events = await journal.events("preserve-draft");
  assert.equal((await protection.status(legacy)).mode, "password");
  assert.equal(await protection.credentials(legacy, PASSWORD), PASSWORD);
  await assert.rejects(protection.credentials(legacy));
  assert.deepEqual(await journal.vault(), legacy);
  const updated = await protection.setLock(legacy, PASSWORD, PIN);
  assert.equal(updated.address, legacy.address);
  assert.equal(updated.created_at, legacy.created_at);
  assert.equal(updated.backup_confirmed, legacy.backup_confirmed);
  assert.notEqual(updated.vault_hex, legacy.vault_hex); assert.notEqual(updated.salt, legacy.salt);
  await assert.rejects(actualCore.authenticate(updated, PASSWORD));
  await assert.rejects(protection.credentials(legacy, PASSWORD), /changed/);
  assert.equal(await actualCore.reveal(updated, await protection.credentials(updated, PIN)), PUBLIC_FIXTURE);
  assert.deepEqual(await journal.events("preserve-draft"), events);
  assert.equal((await journal.get("preserve-draft")).phase, "prepared");
});

test("legacy migration does not mark unfinished recovery verification as completed", async () => {
  const { journal, protection } = await empty();
  const legacy = { ...coreVault(PASSWORD, PUBLIC_FIXTURE).vault, backup_confirmed: false };
  await journal.saveVault(legacy);
  const updated = await protection.setLock(legacy, PASSWORD, null);
  assert.equal(updated.backup_confirmed, false);
  assert.equal(updated.address, legacy.address); assert.equal(updated.created_at, legacy.created_at);
  assert.equal(await actualCore.reveal(updated, await protection.credentials(updated)), PUBLIC_FIXTURE);
  await assert.rejects(journal.recordPrepared(prepare("not-backed-up")), /backup/);
});

test("failed legacy migration rolls back the re-encryption and preserves the existing password", async () => {
  const { journal, protection } = await empty();
  const legacy = { ...coreVault(PASSWORD, PUBLIC_FIXTURE).vault, backup_confirmed: false };
  await journal.saveVault(legacy);
  const restoreWrites = abortProtectionWrite(journal);
  try { await assert.rejects(protection.setLock(legacy, PASSWORD, PIN), /commit|abort/i); } finally { restoreWrites(); }
  assert.deepEqual(await journal.vault(), legacy);
  assert.equal(await raw(journal, "protection"), undefined);
  assert.equal(await protection.credentials(legacy, PASSWORD), PASSWORD);
});

test("concurrent settings updates compare-and-swap rather than silently overwrite", async () => {
  const { journal, protection } = await empty();
  const vault = await protection.restore(PUBLIC_FIXTURE);
  const changes = await Promise.allSettled([protection.setLock(vault, "", PIN), protection.setLock(vault, "", OTHER_PIN)]);
  assert.equal(changes.filter(result => result.status === "fulfilled").length, 1);
  const winner = changes.findIndex(result => result.status === "fulfilled");
  assert.equal((await protection.status(vault)).mode, "pin");
  assert.equal(await actualCore.reveal(vault, await protection.credentials(vault, winner === 0 ? PIN : OTHER_PIN)), PUBLIC_FIXTURE);
  assert.equal((await raw(journal, "protection") as VaultProtectionRecord).revision, 2);
});

test("cancelling creation or a lock change before commit leaves persisted state untouched", async () => {
  let allowed = true;
  const core = { ...actualCore, create: async (password: string) => { const result = await actualCore.create(password); allowed = false; return result; } };
  const { journal, protection } = await empty(core);
  await assert.rejects(protection.create(() => allowed), /cancelled/);
  assert.equal(await journal.vault(), null);
  allowed = true;
  const vault = await protection.restore(PUBLIC_FIXTURE);
  const cancellingCore = { ...actualCore, deriveProtectionKey: async (pin: string, salt: string) => {
    const result = await actualCore.deriveProtectionKey!(pin, salt); allowed = false; return result;
  } };
  await assert.rejects(new WalletProtection(cancellingCore, journal).setLock(vault, "", PIN, () => allowed), /cancelled/);
  assert.equal((await protection.status(vault)).mode, "off");
});

test("cancelling an authorized PIN operation cannot return a credential or reset its reserved attempt", async () => {
  const { journal, protection } = await empty();
  const vault = await protection.setLock(await protection.restore(PUBLIC_FIXTURE), "", PIN);
  let allowed = true;
  const cancellingCore = { ...actualCore, deriveProtectionKey: async (pin: string, salt: string) => {
    const result = await actualCore.deriveProtectionKey!(pin, salt); allowed = false; return result;
  } };
  await assert.rejects(new WalletProtection(cancellingCore, journal).credentials(vault, PIN, () => allowed), /cancelled/);
  assert.equal((await protection.status(vault)).failed_attempts, 1);
});

test("missing, mismatched or exportable device protection never falls back to an unlocked wallet", async () => {
  const { journal, protection } = await empty();
  const vault = await protection.restore(PUBLIC_FIXTURE);
  const original = await raw(journal, "protection") as VaultProtectionRecord;
  await mutate(journal, "protection", store => store.delete("primary"));
  await assert.rejects(protection.status(vault), /device key.*missing/);
  await assert.rejects(protection.credentials(vault), /device key.*missing/);
  await mutate(journal, "protection", store => store.put({ ...original, vault_binding: "other-vault" }));
  await assert.rejects(protection.credentials(vault), /could not be validated/);
  const exportable = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, true, ["encrypt", "decrypt"]);
  await mutate(journal, "protection", store => store.put({ ...original, device_key: exportable }));
  await assert.rejects(protection.credentials(vault), /could not be validated/);
  await mutate(journal, "protection", store => store.put(original));
  const unmarked = { ...vault }; delete unmarked.protection;
  await mutate(journal, "vault", store => store.put(unmarked));
  await assert.rejects(protection.credentials(unmarked), /do not match/);
});

test("a concurrent backup confirmation survives an app-lock change", async () => {
  const { journal, protection } = await empty();
  const created = await protection.create();
  const confirmingCore = { ...actualCore, deriveProtectionKey: async (pin: string, salt: string) => {
    await journal.confirmBackup(created.vault);
    return actualCore.deriveProtectionKey!(pin, salt);
  } };
  const result = await new WalletProtection(confirmingCore, journal).setLock(created.vault, "", PIN);
  assert.equal(result.backup_confirmed, true);
  assert.equal((await journal.vault())!.backup_confirmed, true);
  assert.equal(await actualCore.reveal(result, await protection.credentials(result, PIN)), created.mnemonic);
});
