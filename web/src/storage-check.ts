/** Public fixture only. This module never imports or opens the wallet journal. */
const SAMPLE = "Local Wallet public encrypted-storage diagnostic v1";
const STORE = "public_probe";
const STEP_TIMEOUT = 8_000;
type Stage = "availability" | "create" | "commit" | "reopen" | "read" | "decrypt" | "export" | "cleanup";
const failureText: Record<Stage, string> = {
  availability: "This browser does not provide the encryption and storage APIs required by the wallet.",
  create: "A separate temporary storage check could not be created. Your wallet was not opened.",
  commit: "The browser could not durably save the encrypted diagnostic and its non-extractable key.",
  reopen: "The temporary diagnostic database could not be reopened.",
  read: "The reopened encryption key or diagnostic data did not match the required format.",
  decrypt: "The stored encryption key could not decrypt the public diagnostic after reopening.",
  export: "The browser did not demonstrate the required key-export restriction.",
  cleanup: "The temporary diagnostic database could not be removed. It contains only public test data.",
};
function bounded<T>(promise: Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Diagnostic step timed out.")), STEP_TIMEOUT);
    promise.then(value => { clearTimeout(timer); resolve(value); }, () => { clearTimeout(timer); reject(new Error("Diagnostic step failed.")); });
  });
}
function committed(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { try { tx.abort(); } catch { /* already closed */ } reject(new Error("Diagnostic storage timed out.")); }, STEP_TIMEOUT);
    tx.oncomplete = () => { clearTimeout(timer); resolve(); };
    tx.onabort = () => { clearTimeout(timer); reject(new Error("Diagnostic storage did not commit.")); };
    tx.onerror = () => { /* Transaction abort, not request success, is authoritative. */ };
  });
}
export interface EncryptedStorageCheck {
  durable_commit: true;
  reopened_decryption: true;
  non_extractable_key: true;
  key_export_blocked: true;
  temporary_data_removed: true;
}

export async function checkEncryptedStorage(): Promise<EncryptedStorageCheck> {
  let stage: Stage = "availability", failure: string | null = null;
  let db: IDBDatabase | null = null, created = false, name = "";
  const open = (creating: boolean): Promise<IDBDatabase> => new Promise((resolve, reject) => {
    const request = indexedDB.open(name, 1);
    let settled = false;
    const fail = () => { if (!settled) { settled = true; clearTimeout(timer); try { request.transaction?.abort(); } catch { /* already finished */ } reject(new Error("Diagnostic database could not open.")); } };
    const timer = setTimeout(fail, STEP_TIMEOUT);
    request.onupgradeneeded = event => {
      // Never touch an existing database, or silently recreate a missing probe.
      if (!creating || event.oldVersion !== 0 || settled) { request.transaction?.abort(); fail(); return; }
      created = true;
      request.result.createObjectStore(STORE, { keyPath: "id" });
    };
    request.onerror = fail; request.onblocked = fail;
    request.onsuccess = () => {
      if (settled || (creating && !created)) { request.result.close(); fail(); return; }
      settled = true; clearTimeout(timer);
      request.result.onversionchange = () => request.result.close();
      resolve(request.result);
    };
  });
  try {
    if (!globalThis.indexedDB || !globalThis.crypto?.subtle || !globalThis.crypto?.randomUUID) throw new Error("Required API unavailable.");
    name = `local-wallet-public-storage-check-${crypto.randomUUID()}`;
    stage = "create";
    const key = await bounded(crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]));
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const plaintext = new TextEncoder().encode(SAMPLE);
    const encrypted = await bounded(crypto.subtle.encrypt({ name: "AES-GCM", iv, tagLength: 128 }, key, plaintext));
    db = await open(true);
    stage = "commit";
    const write = db.transaction(STORE, "readwrite", { durability: "strict" });
    if (write.durability !== "strict") { write.abort(); throw new Error("Strict durability unavailable."); }
    const complete = committed(write);
    try {
      write.objectStore(STORE).add({ id: "public", key, iv, encrypted });
      await complete;
    } catch {
      try { write.abort(); } catch { /* already completed or aborted */ }
      await complete.catch(() => undefined);
      throw new Error("Diagnostic storage did not commit.");
    }
    db.close(); db = null;
    stage = "reopen"; db = await open(false);
    stage = "read";
    const read = db.transaction(STORE), finished = committed(read);
    const value = new Promise<unknown>((resolve, reject) => {
      const request = read.objectStore(STORE).get("public");
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(new Error("Diagnostic read failed."));
    });
    const [stored] = await Promise.all([value, finished]);
    const probe = stored as { id?: string; key?: CryptoKey; iv?: Uint8Array<ArrayBuffer>; encrypted?: ArrayBuffer };
    const savedKey = probe?.key;
    if (probe?.id !== "public" || !savedKey || savedKey.type !== "secret" || savedKey.extractable !== false
      || savedKey.algorithm?.name !== "AES-GCM" || (savedKey.algorithm as AesKeyAlgorithm).length !== 256
      || [...savedKey.usages].sort().join(",") !== "decrypt,encrypt" || !(probe.iv instanceof Uint8Array) || probe.iv.byteLength !== 12
      || !(probe.encrypted instanceof ArrayBuffer) || probe.encrypted.byteLength !== plaintext.byteLength + 16) throw new Error("Stored diagnostic mismatch.");
    stage = "decrypt";
    const decrypted = new Uint8Array(await bounded(crypto.subtle.decrypt({ name: "AES-GCM", iv: probe.iv, tagLength: 128 }, savedKey, probe.encrypted)));
    if (new TextDecoder("utf-8", { fatal: true }).decode(decrypted) !== SAMPLE) throw new Error("Public diagnostic mismatch.");
    stage = "export";
    let blocked = false;
    // Preserve only the expected error classification, never native error text.
    await bounded(crypto.subtle.exportKey("raw", savedKey).then(bytes => { new Uint8Array(bytes).fill(0); }, error => {
      if (error instanceof DOMException && error.name === "InvalidAccessError") blocked = true;
      else throw new Error("Unexpected key-export behavior.");
    }));
    if (!blocked) throw new Error("Key export was not blocked.");
  } catch { failure = failureText[stage]; }
  finally {
    db?.close();
    if (created) {
      try {
        await bounded(new Promise<void>((resolve, reject) => {
          // Only the unique database created by this invocation is eligible.
          const remove = indexedDB.deleteDatabase(name);
          remove.onsuccess = () => resolve();
          remove.onerror = () => reject(new Error("Diagnostic cleanup failed."));
          remove.onblocked = () => reject(new Error("Diagnostic cleanup was blocked."));
        }));
      } catch { failure = failure ? `${failure} ${failureText.cleanup}` : failureText.cleanup; }
    }
  }
  if (failure) throw new Error(failure);
  return { durable_commit: true, reopened_decryption: true, non_extractable_key: true, key_export_blocked: true, temporary_data_removed: true };
}
