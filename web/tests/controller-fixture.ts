import { build } from "esbuild";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createContext, runInContext } from "node:vm";
import { fixtureBrowser } from "./controller-dom";
import type { ActivityResult, BalanceSnapshot, ChainActivity, ChainId, FeePreview, PaymentRecord, PreparedPayment } from "../src/types";

export const CONTROLLER_ADDRESS = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
export const CONTROLLER_RECIPIENT = "0x1111111111111111111111111111111111111111";
const fixtureVault = { id: "primary", version: 1, kdf: "argon2id-64m-t3-p1", salt: "00".repeat(16), vault_hex: "0x00", address: CONTROLLER_ADDRESS, created_at: 1_700_000_000, backup_confirmed: true, protection: "device-key-v1" };
const registry = ([8453, 1] as const).map(chain_id => ({ chain_id, network: chain_id === 8453 ? "Base" : "Ethereum",
  rpc_url: "https://fixture.invalid", broadcast_url: "https://fixture.invalid", explorer_url: "https://fixture.invalid",
  assets: [{ symbol: "USDC", decimals: 6, contract: "0x" + (chain_id === 8453 ? "3" : "1").repeat(40) }, { symbol: "EURC", decimals: 6, contract: "0x" + (chain_id === 8453 ? "4" : "2").repeat(40) }, { symbol: "ETH", decimals: 18, contract: null }] }));
const copy = <T>(value: T): T => JSON.parse(JSON.stringify(value));

export function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
export async function settle(predicate: () => boolean = () => true): Promise<void> {
  for (let step = 0; step < 100; step++) { await Promise.resolve(); if (step > 20 && predicate()) return; }
  if (!predicate()) throw new Error("Controller fixture did not settle.");
}
export function snapshot(chain_id: ChainId, usdc = "10000", block_number = 100): BalanceSnapshot {
  return { chain_id, address: CONTROLLER_ADDRESS, block_number, block_hash: "0x" + "a".repeat(64), balances: { USDC: usdc, EURC: "0", ETH: "1000000000000000" }, observed_at: 1_700_000_000 };
}
export function freshBalance(chain: ChainId, usdc = "10000") {
  return { status: "fresh", snapshot: snapshot(chain, usdc), provider: "Public fixture", observed_at: 1_700_000_000, error: null };
}
export function quote(chain_id: ChainId, asset: "USDC" | "EURC" | "ETH" = "USDC"): FeePreview {
  return { chain_id, asset, estimated_execution_fee_wei: "100", l1_fee_estimate_wei: "0", operator_fee_estimate_wei: "0",
    estimated_total_fee_wei: "100", max_execution_fee_wei: "1000", fee_reserve_wei: "1000", gas_limit: 100000,
    max_fee_per_gas: "1", max_priority_fee_per_gas: "1", provider: "Public fixture", estimated_at: Math.floor(Date.now() / 1000), estimate_source: "simulation", warnings: [], shortages: [] };
}
export function prepared(chain_id: ChainId, id = `fixture-${chain_id}`, asset = "USDC", amount = "0.01", to = CONTROLLER_RECIPIENT): PreparedPayment {
  return { request: { intent_id: id, chain_id, from: CONTROLLER_ADDRESS, to, asset, amount, nonce: 0, gas_limit: 100000,
    max_fee_per_gas: "1", max_priority_fee_per_gas: "1", l1_fee_estimate_wei: "0", operator_fee_estimate_wei: "0" },
    amount_units: "10000", max_fee_wei: "1000", estimated_total_fee_wei: "100", fee_reserve_wei: "1000", unsigned_transaction: "public-fixture-only", approval_digest: "0x" + "b".repeat(64), created_at: 1_700_000_000, expires_at: Math.floor(Date.now() / 1000) + 120 } as PreparedPayment;
}
export function payment(chain: ChainId, id = `fixture-${chain}`, phase: PaymentRecord["phase"] = "prepared", hash = "0x" + "c".repeat(64)): PaymentRecord {
  const draft = prepared(chain, id);
  const signed = phase === "prepared" || phase === "cancelled" ? null : { ...draft.request, hash, raw_transaction: "public-fixture-only" };
  return { intent_id: id, account: `${chain}:${CONTROLLER_ADDRESS.toLowerCase()}`, nonce_key: `${chain}:${CONTROLLER_ADDRESS.toLowerCase()}:0`, prepared: draft, signed, phase, execution: "unknown", inclusion: "unseen", receipt: null, receipt_proof: null, resolution_note: null, created_at: 1_700_000_000, updated_at: 1_700_000_000, revision: 0 } as PaymentRecord;
}
export function activity(chain_id: ChainId, hash = "0x" + "d".repeat(64), direction: ChainActivity["direction"] = "received"): ChainActivity {
  return { id: `${chain_id}:${hash}:0:${direction}`, chain_id, address: CONTROLLER_ADDRESS, direction, asset: "USDC", asset_contract: registry.find(network => network.chain_id === chain_id)!.assets[0].contract,
    amount_units: "10000", amount: "0.01", from: direction === "received" ? CONTROLLER_RECIPIENT : CONTROLLER_ADDRESS, to: direction === "received" ? CONTROLLER_ADDRESS : CONTROLLER_RECIPIENT,
    transaction_hash: hash, block_number: 100, block_hash: "0x" + "a".repeat(64), log_index: 0, timestamp: 1_700_000_001, inclusion: "finalized" };
}
const emptyActivity = (): ActivityResult => ({ items: [], coverage: "recent", complete: false, from_block: 1, to_block: 100, provider: "Public fixture", observed_at: 1_700_000_000, warnings: [], error: null });

function services(qr: string[]) {
  const snapshots = new Map<ChainId, BalanceSnapshot>();
  const records = new Map<string, PaymentRecord>();
  const activities = new Map<ChainId, ChainActivity[]>();
  const writes: BalanceSnapshot[] = [], balanceCalls: ChainId[] = [], previewCalls: ChainId[] = [], signedCalls: ChainId[] = [];
  const methods = {
    balanceStatus: async (chain: ChainId, _address: string): Promise<any> => freshBalance(chain, "0"),
    preview: async (chain: ChainId, draft: any, _signal: AbortSignal): Promise<FeePreview> => quote(chain, draft.asset),
    refreshActivity: async (_chain: ChainId, _address: string): Promise<ActivityResult> => emptyActivity(),
    lookupTransaction: async (_chain: ChainId, _address: string, _hash: string): Promise<ActivityResult> => emptyActivity(),
    sign: async (chain: ChainId, draft: PreparedPayment, authorized: () => Promise<boolean>) => {
      if (!await authorized()) throw new Error("Fixture signing cancelled.");
      if (draft.request.chain_id !== chain) throw new Error("Fixture signing network mismatch.");
      signedCalls.push(chain);
      const result = payment(chain, draft.request.intent_id, "pending"); result.prepared = draft; records.set(result.intent_id, result);
      return { record: copy(result), acknowledged: true };
    },
  };
  const journal = {
    vault: async () => copy(fixtureVault), protectionState: async () => ({ vault: copy(fixtureVault), protection: { mode: "off", revision: 1 } }),
    assertProtectionCurrent: async () => {},
    latestBalance: async (chain: ChainId) => copy(snapshots.get(chain) ?? null),
    recordBalance: async (value: BalanceSnapshot) => { writes.push(copy(value)); snapshots.set(value.chain_id, copy(value)); },
    list: async (chain: ChainId) => [...records.values()].filter(record => record.prepared.request.chain_id === chain).map(copy),
    listActivity: async (chain: ChainId) => copy(activities.get(chain) ?? []),
    get: async (id: string) => { const record = records.get(id); if (!record) throw new Error("Unknown fixture record."); return copy(record); },
    cancel: async (id: string) => { const record = records.get(id)!; const cancelled = { ...record, phase: "cancelled" as const }; records.set(id, cancelled); return copy(cancelled); },
  };
  return { registry, qr, snapshots, records, activities, writes, balanceCalls, previewCalls, signedCalls, methods, journal,
    network: {
      balanceStatus: (chain: ChainId, owner: string) => { balanceCalls.push(chain); return methods.balanceStatus(chain, owner); },
      preview: (chain: ChainId, draft: any, signal: AbortSignal) => { previewCalls.push(chain); return methods.preview(chain, draft, signal); },
    },
    engine: {
      refreshActivity: (chain: ChainId, owner: string) => methods.refreshActivity(chain, owner),
      lookupTransaction: (chain: ChainId, owner: string, hash: string) => methods.lookupTransaction(chain, owner, hash),
      reconcile: async (chain: ChainId, id: string) => { const record = await journal.get(id); if (record.prepared.request.chain_id !== chain) throw new Error("Fixture reconciliation network mismatch."); return record; },
      prepare: async (chain: ChainId, _vault: unknown, to: string, asset: string, amount: string) => {
        const draft = prepared(chain, `review-${chain}`, asset, amount, to); const record = payment(chain, draft.request.intent_id); record.prepared = draft; records.set(record.intent_id, record); return copy(draft);
      },
      signAndSubmit: (chain: ChainId, _vault: unknown, _credential: string, draft: PreparedPayment, authorized: () => Promise<boolean>) => methods.sign(chain, draft, authorized),
      retry: async (chain: ChainId, id: string, authorized: () => Promise<boolean>) => { if (!await authorized()) throw new Error("Fixture retry cancelled."); const record = await journal.get(id); if (record.prepared.request.chain_id !== chain) throw new Error("Fixture retry network mismatch."); return { record, acknowledged: true }; },
    },
  };
}

const serviceModule = `const h = globalThis.__controllerFixture;
export class CoreClient { async registry() { return { networks: h.registry }; } cancelAll() {} }
export class Journal { static async open() { return h.journal; } }
export const isActive = record => record.phase !== 'cancelled' && record.phase !== 'finalized';
export class WalletProtection { async status() { return { mode: 'off', retry_at: 0, failed_attempts: 0 }; } async credentials() { return 'public-fixture-not-a-real-credential'; } }
export class NetworkClient { constructor(network) { this.network = network; } balanceStatus(owner) { return h.network.balanceStatus(this.network.chain_id, owner); } preview(draft, signal) { return h.network.preview(this.network.chain_id, draft, signal); } }
export class WalletEngine { constructor(_core, _journal, rpc) { this.chain = rpc.network.chain_id; }
refreshActivity(...args) { return h.engine.refreshActivity(this.chain,...args); }
lookupTransaction(...args) { return h.engine.lookupTransaction(this.chain,...args); }
prepare(...args) { return h.engine.prepare(this.chain,...args); }
reconcile(...args) { return h.engine.reconcile(this.chain,...args); }
signAndSubmit(...args) { return h.engine.signAndSubmit(this.chain,...args); }
retry(...args) { return h.engine.retry(this.chain,...args); } }
export default { toCanvas(_canvas, uri) { h.qr.push(uri); return Promise.resolve(); } };`;

let bundle: Promise<string> | undefined;
async function controllerBundle(): Promise<string> {
  bundle ??= (async () => {
    const file = new URL("../src/main.ts", import.meta.url);
    const source = await readFile(file, "utf8");
    if (!/void start\(\);\s*$/.test(source)) throw new Error("Controller boot boundary changed; update the fixture explicitly.");
    // Only the in-memory test bundle exposes controller actions. Production files are unchanged.
    const exposed = source.replace(/void start\(\);\s*$/, "") + `\nexport const controller = {
      start, render, lock, resumeFromStorage, refreshWallet, refreshBalances, refreshActivities, loadSaved,
      beginTransfer, navigate, estimateFee, openPayment, displayActivities, lookupTransaction,
      inspect: () => ({ chain, route, unlocked, busy, epoch, preview, reviewed, currentPayment, currentActivity, sendDraft: {...sendDraft},
        views: portfolioViews(), states: [...portfolio].map(([chainId,value]) => ({chainId,...value})) }),
    };`;
    const result = await build({ stdin: { contents: exposed, resolveDir: fileURLToPath(new URL("../src", import.meta.url)), sourcefile: "main-controller-fixture.ts", loader: "ts" }, bundle: true, format: "cjs", platform: "node", target: "es2022", write: false,
      plugins: [{ name: "controller-boundaries", setup(builder) {
        builder.onResolve({ filter: /^(?:\.\/(?:core-client|journal|wallet-protection|network|engine)|qrcode)$/ }, args => ({ path: args.path, namespace: "controller-mocks" }));
        builder.onLoad({ filter: /.*/, namespace: "controller-mocks" }, () => ({ contents: serviceModule, loader: "js" }));
      } }] });
    return result.outputFiles[0].text;
  })();
  return bundle;
}

export async function openController(configure?: (fixture: ReturnType<typeof services>) => void) {
  const browser = fixtureBrowser(), fixture = services(browser.qr);
  configure?.(fixture);
  const module = { exports: {} as any };
  const context = createContext({ ...browser.globals, __controllerFixture: fixture, module, exports: module.exports });
  runInContext(await controllerBundle(), context, { timeout: 5_000 });
  const controller = module.exports.controller;
  await controller.start();
  await settle(() => controller.inspect().unlocked && !controller.inspect().busy);
  return { ...browser, fixture, controller, inspect: () => copy(controller.inspect()), async idle() { await settle(() => !controller.inspect().busy); } };
}
