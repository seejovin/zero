import QRCode from "qrcode";
import { CoreClient } from "./core-client";
import { Journal, isActive } from "./journal";
import { WalletEngine } from "./engine";
import { WalletProtection, type WalletLockStatus } from "./wallet-protection";
import { NetworkClient } from "./network";
import { address as validateAddress, constantJson, formatUnits, networkAsset, receiveUri, units } from "./units";
import { button, card, detail, el, field, icon, info, input, pill, walletHeader, walletOverview, patchWalletOverview, portfolioConnectionStrip, assetPicker, patchAssetPicker, connectionStrip, activityList, feePanel, assetBadge, chainBadge, assetOverview, patchAssetOverview, openingWordmark } from "./ui";
import type { ActivityItemView, ConnectionProps, PortfolioNetworkView } from "./ui";
import { DEFAULT_DISPLAY_NAMES, readDisplayNames, saveDisplayNames, type DisplayNames } from "./display-preferences";
import { validateAssetRef, type AssetRef } from "./portfolio";
import type { ActivityResult, Asset, BalanceSnapshot, ChainActivity, ChainId, FeePreview, Network, PaymentRecord, PreparedPayment, Vault } from "./types";

type Route = "welcome" | "create" | "restore" | "unlock" | "backup" | "challenge" | "assets" | "asset" | "activity" | "profile" | "rename" | "settings" | "app-lock" | "set-pin" | "disable-lock" | "choose-deposit" | "choose-send" | "send" | "review" | "receive" | "reveal" | "words" | "transaction" | "retry" | "chain-transaction";
const mount = document.getElementById("app")!;
const core = new CoreClient();
let journal: Journal;
let protection: WalletProtection;
let lockStatus: WalletLockStatus = { mode: "password", retry_at: 0, failed_attempts: 0 };
let networks: Network[] = [];
const clients = new Map<ChainId, NetworkClient>();
let vault: Vault | null = null;
let chain: ChainId = 8453;
let unlocked = false;
let route: Route = "welcome";
let renderedRoute: Route | null = null;
let busy = "";
let error = "";
let notice = "";
let phrase = "";
let challenge: number[] = [];
let reviewed: PreparedPayment | null = null;
let reviewedPreview: FeePreview | null = null;
let currentPayment: PaymentRecord | null = null;
let currentActivity: ChainActivity | null = null;
interface NetworkState {
  records: PaymentRecord[]; chainItems: ChainActivity[]; activityResult: ActivityResult | null; activityError: string;
  snapshot: BalanceSnapshot | null; connectionState: ConnectionProps["state"]; connectionError: string | null; connectionProvider: string | null;
  refreshedAt: number | null; refreshingBalance: boolean; refreshingActivity: boolean; lastActivityRefresh: number;
  balanceGeneration: number; activityGeneration: number;
}
const portfolio = new Map<ChainId, NetworkState>();
function state(id: ChainId = chain): NetworkState {
  let value = portfolio.get(id);
  if (!value) {
    value = { records: [], chainItems: [], activityResult: null, activityError: "", snapshot: null, connectionState: "idle", connectionError: null, connectionProvider: null, refreshedAt: null, refreshingBalance: false, refreshingActivity: false, lastActivityRefresh: 0, balanceGeneration: 0, activityGeneration: 0 };
    portfolio.set(id, value);
  }
  return value;
}
let displayNames: DisplayNames = { ...DEFAULT_DISPLAY_NAMES };
let epoch = 0;
let lastInteraction = Date.now();
let sendDraft = { to: "", amount: "", asset: "USDC" as Asset };
const draftByChain = new Map<ChainId, typeof sendDraft>();
let receiveAsset: Asset = "USDC";
let selectedAsset: Asset = "USDC";
let hideBalances = false;
let preview: FeePreview | null = null;
let previewKey = "";
let feeLoading = false;
let feeError = "";
let feeGeneration = 0;
let feeTimer: ReturnType<typeof setTimeout> | null = null;
let feeAbort: AbortController | null = null;
let receiveBack: Route = "assets";
let lookupHash = "";
let lookupChain: ChainId = 8453;
let lookupRunning = false;
let lookupQueuedHash: string | null = null;
let lookupMessage = "";
const protectionChanges = typeof BroadcastChannel === "function" ? new BroadcastChannel("local-wallet-protection-v1") : null;

const network = (id: ChainId = chain) => networks.find(n => n.chain_id === id)!;
function rpc(id: ChainId = chain): NetworkClient {
  let client = clients.get(id);
  if (!client) { client = new NetworkClient(network(id)); clients.set(id, client); }
  return client;
}
const engine = (id: ChainId = chain) => new WalletEngine(core, journal, rpc(id));
const displayName = (symbol: Asset) => ({ USDC: "USD Coin", EURC: "Euro Coin", ETH: "Ether" })[symbol];
const assetIcon = assetBadge;
const chainMark = chainBadge;
const netLabel = (id: ChainId = chain) => id === 8453 ? "Base" : "Ethereum";
const canRead = () => Boolean(unlocked && vault?.backup_confirmed && document.visibilityState !== "hidden");
function navigate(next: Route): void {
  if (busy) return;
  if (!unlocked && vault && next !== "unlock") return;
  if (unlocked && vault && !vault.backup_confirmed && !["backup", "challenge"].includes(next)) next = "backup";
  error = ""; notice = "";
  if (route === "words" && next !== "words") phrase = "";
  route = next;
  render();
  if (next === "send" && validDraft() && !currentPreview()) scheduleEstimate();
}
function updateBusy(): void {
  const overlay = document.getElementById("busy-status");
  if (overlay) { overlay.replaceChildren(...(busy ? [el("span", { class: "spinner" }), document.createTextNode(busy)] : [])); overlay.classList.toggle("visible", Boolean(busy)); }
  for (const element of document.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>("button, input, select, textarea")) {
    if (element.dataset.lock === "true") continue;
    if (busy) { if (!element.disabled) element.dataset.busyDisabled = "true"; element.disabled = true; }
    else if (element.dataset.busyDisabled === "true") { element.disabled = false; delete element.dataset.busyDisabled; }
  }
}
function renderMessages(): void {
  const messages = document.getElementById("messages");
  if (messages) messages.replaceChildren(...[error ? info(error, "warning") : null, notice ? info(notice, "success") : null].filter((node): node is HTMLElement => node !== null));
}
async function run(label: string, task: (authorized: () => boolean) => Promise<void>): Promise<void> {
  if (busy) return;
  const operationEpoch = epoch;
  busy = label; error = ""; notice = ""; renderMessages(); updateBusy();
  try { await task(() => operationEpoch === epoch); }
  catch (failure) {
    if (operationEpoch === epoch) {
      error = failure instanceof Error ? failure.message : "This operation could not be completed.";
      if (!vault && journal) {
        try { vault = await journal.vault(); if (vault) { lockStatus = await protection.status(vault); route = "unlock"; } } catch { /* Preserve the original error. */ }
      }
    }
  } finally {
    if (operationEpoch === epoch) {
      busy = ""; render();
      if (lookupQueuedHash && canRead() && !state(lookupChain).refreshingActivity && !lookupRunning) void lookupTransaction();
    } else if (!busy && !unlocked && document.visibilityState !== "hidden") {
      // A creation commit may have completed just as the app went to the background.
      // Re-read that wallet; never return to creating a replacement over it.
      void resumeFromStorage();
    }
  }
}
function resetPreview(): void {
  feeGeneration++; feeAbort?.abort(); feeAbort = null; preview = null; previewKey = ""; feeLoading = false; feeError = "";
  if (feeTimer) clearTimeout(feeTimer);
  feeTimer = null;
}
function lock(): void {
  epoch++;
  unlocked = false; phrase = ""; challenge = []; reviewed = null; reviewedPreview = null; currentPayment = null; currentActivity = null;
  portfolio.clear(); displayNames = { ...DEFAULT_DISPLAY_NAMES };
  lookupRunning = false; lookupQueuedHash = null; lookupHash = ""; lookupMessage = "";
  sendDraft = { to: "", amount: "", asset: "USDC" }; draftByChain.clear(); resetPreview();
  core.cancelAll(); busy = ""; error = ""; notice = "";
  for (const field of document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input, textarea")) field.value = "";
  route = vault ? "unlock" : "welcome";
  render();
}
/** The home has no selected network. This context belongs to the explicit transfer or detail only. */
function selectOperationChain(id: ChainId): void {
  if (id !== 1 && id !== 8453) throw new Error("Choose a supported asset and network.");
  if (chain === id) return;
  draftByChain.set(chain, { ...sendDraft });
  chain = id;
  reviewed = null; reviewedPreview = null; currentPayment = null; currentActivity = null;
  sendDraft = draftByChain.get(id) ?? { to: "", amount: "", asset: "USDC" };
  resetPreview();
}
async function loadSaved(authorized: () => boolean): Promise<void> {
  if (!vault) return;
  const owner = vault.address;
  await Promise.allSettled(networks.flatMap(entry => {
    const selected = entry.chain_id, view = state(selected), activityToken = view.activityGeneration;
    return [
      journal.latestBalance(selected, owner).then(balance => {
        if (!authorized() || !balance || view.snapshot) return;
        view.snapshot = balance; view.connectionState = "stale"; view.refreshedAt = balance.observed_at; patchLive();
      }),
      journal.list(selected, owner).then(payments => {
        if (authorized() && activityToken === view.activityGeneration) { view.records = payments; patchLive(); }
      }).catch(() => {
        if (authorized()) { error = "Saved transaction history could not be validated. Sending is blocked until the journal is checked."; renderMessages(); }
      }),
      journal.listActivity(selected, owner).then(items => {
        if (authorized()) { mergeActivity(items, selected, true); patchLive(); }
      }).catch(() => {
        if (authorized()) { view.activityError = "Saved chain activity could not be validated. A fresh network check is required."; patchLive(); }
      }),
    ];
  }));
}
function hydrateAndRefresh(authorized: () => boolean): void {
  // Both networks hydrate independently; an unavailable chain is never shown as zero.
  if (vault) displayNames = readDisplayNames(vault.address);
  void loadSaved(authorized);
  startRefreshSoon();
}
function startRefreshSoon(): void {
  setTimeout(() => { if (canRead()) refreshWallet(true); }, 0);
}
function refreshWallet(forceActivity = false): void {
  if (!canRead() || busy) return;
  for (const entry of networks) {
    void refreshBalances(entry.chain_id);
    if (entry.chain_id === lookupChain && lookupQueuedHash && !state(entry.chain_id).refreshingActivity && !lookupRunning) { void lookupTransaction(); continue; }
    if (forceActivity || Date.now() - state(entry.chain_id).lastActivityRefresh >= 45_000) void refreshActivities(entry.chain_id);
  }
}
async function refreshBalances(selected: ChainId = chain): Promise<void> {
  const view = state(selected);
  if (!canRead() || view.refreshingBalance || !vault) return;
  const token = ++view.balanceGeneration, session = epoch, owner = vault.address;
  const authorized = () => token === view.balanceGeneration && session === epoch && unlocked;
  view.refreshingBalance = true;
  if (!view.snapshot) view.connectionState = "loading";
  patchLive();
  try {
    const result = await rpc(selected).balanceStatus(owner);
    if (!authorized()) return;
    view.connectionProvider = result.provider;
    if (result.status !== "fresh" || !result.snapshot) throw new Error(result.error ?? "The network could not return a fresh balance.");
    if (result.snapshot.chain_id !== selected || result.snapshot.address.toLowerCase() !== owner.toLowerCase()) throw new Error("A balance response did not match this account and network.");
    await journal.recordBalance(result.snapshot);
    const saved = await journal.latestBalance(selected, owner);
    if (!authorized()) return;
    if (!saved || saved.block_hash !== result.snapshot.block_hash || saved.block_number !== result.snapshot.block_number || constantJson(saved.balances) !== constantJson(result.snapshot.balances)) throw new Error("A stale balance response was preserved as history, not shown as current.");
    view.snapshot = saved; view.connectionState = "live"; view.connectionError = null; view.refreshedAt = result.observed_at;
    if (selected === chain && route === "send" && validDraft() && (preview?.shortages.length || !currentPreview())) scheduleEstimate();
  } catch (failure) {
    if (authorized()) { view.connectionState = view.snapshot ? "stale" : "unavailable"; view.connectionError = failure instanceof Error ? failure.message : "The network could not be reached."; }
  } finally {
    if (authorized()) { view.refreshingBalance = false; patchLive(); }
  }
}
function mergeActivity(items: ChainActivity[], selected: ChainId, keepNewer = false): void {
  const view = state(selected), map = new Map(view.chainItems.map(item => [item.id, item]));
  for (const item of items) if (item.chain_id === selected && item.address.toLowerCase() === vault?.address.toLowerCase() && (!keepNewer || !map.has(item.id))) map.set(item.id, item);
  view.chainItems = [...map.values()].sort((a, b) => b.timestamp - a.timestamp || b.block_number - a.block_number);
  if (currentActivity?.chain_id === selected) currentActivity = map.get(currentActivity.id) ?? currentActivity;
}
async function refreshActivities(selected: ChainId = chain): Promise<void> {
  const view = state(selected);
  if (!canRead() || view.refreshingActivity || (selected === lookupChain && (lookupRunning || lookupQueuedHash)) || !vault) return;
  const token = ++view.activityGeneration, session = epoch, owner = vault.address;
  const authorized = () => token === view.activityGeneration && session === epoch && unlocked;
  const service = engine(selected);
  view.refreshingActivity = true; view.activityError = ""; patchLive();
  try {
    const saved = await journal.list(selected, owner);
    if (!authorized()) return;
    view.records = saved; patchLive();
    // Each network retains its own journal, nonces and pending reconciliation.
    for (const payment of saved.filter(item => item.signed && isActive(item))) {
      if (!authorized()) return;
      try { await service.reconcile(payment.intent_id); }
      catch (failure) { if (authorized()) view.activityError = failure instanceof Error ? failure.message : "A pending transfer could not be reconciled."; }
    }
    if (!authorized()) return;
    const reconciled = await journal.list(selected, owner);
    if (!authorized()) return;
    view.records = reconciled;
    const result: ActivityResult = await service.refreshActivity(owner);
    if (!authorized()) return;
    view.activityResult = result; mergeActivity(result.items, selected);
    if (result.error) view.activityError = result.error;
  } catch (failure) {
    if (authorized()) view.activityError = failure instanceof Error ? failure.message : "Recent transactions could not be loaded.";
  } finally {
    if (authorized()) {
      view.refreshingActivity = false; view.lastActivityRefresh = Date.now();
      if (currentPayment?.prepared.request.chain_id === selected) currentPayment = view.records.find(item => item.intent_id === currentPayment!.intent_id) ?? currentPayment;
      patchLive();
      if (selected === lookupChain && lookupQueuedHash && !busy) void lookupTransaction();
    }
  }
}
function portfolioViews(): PortfolioNetworkView[] {
  return ([8453, 1] as ChainId[]).map(id => {
    const view = state(id), spec = network(id);
    const balances = Object.fromEntries((["USDC", "EURC", "ETH"] as Asset[]).map(symbol => [symbol, view.snapshot ? formatUnits(view.snapshot.balances[symbol], networkAsset(spec, symbol).decimals) : null])) as Record<Asset, string | null>;
    return { chainId: id, balances, state: view.connectionState, refreshing: view.refreshingBalance, updatedAt: view.refreshedAt, blockNumber: view.snapshot?.block_number ?? null, provider: view.connectionProvider, error: view.connectionError };
  });
}
function connectionView(): HTMLElement {
  if (["assets", "activity", "choose-deposit", "choose-send"].includes(route)) return portfolioConnectionStrip({ networks: portfolioViews(), onRefresh: () => refreshWallet(true) });
  const view = state();
  return connectionStrip({ state: view.connectionState, refreshing: view.refreshingBalance, updatedAt: view.refreshedAt, blockNumber: view.snapshot?.block_number ?? null, provider: view.connectionProvider, error: view.connectionError, networkName: netLabel(), onRefresh: () => refreshWallet(true) });
}
function openSend(asset: Asset): void {
  if (sendDraft.asset !== asset) { sendDraft.asset = asset; sendDraft.amount = ""; resetPreview(); }
  reviewed = null; reviewedPreview = null; navigate("send");
}
function beginTransfer(mode: "deposit" | "send", ref?: AssetRef): void {
  if (busy) return;
  if (!ref) { navigate(mode === "deposit" ? "choose-deposit" : "choose-send"); return; }
  const selected = validateAssetRef(ref);
  selectOperationChain(selected.chainId);
  if (mode === "send") openSend(selected.symbol);
  else openReceive(selected.symbol, "choose-deposit");
}
async function copyAddress(id?: ChainId): Promise<void> {
  if (!vault) return;
  try { await navigator.clipboard.writeText(vault.address); notice = id ? `Address copied. Use the ${netLabel(id)} network.` : "Address copied."; error = ""; }
  catch { error = "Copy was unavailable. Select and copy the full address from Deposit."; }
  renderMessages();
}
function overviewProps(activeTab: "assets" | "activity" = "assets"): Parameters<typeof walletOverview>[0] {
  return { address: vault!.address, ...displayNames, networks: portfolioViews(), hidden: hideBalances, activeTab,
    onSend: ref => beginTransfer("send", ref), onReceive: ref => beginTransfer("deposit", ref), onCopy: () => void copyAddress(), onEditNames: () => navigate("rename"),
    onAsset: ref => { if (busy) return; const selected = validateAssetRef(ref); selectOperationChain(selected.chainId); selectedAsset = selected.symbol; navigate("asset"); },
    onTogglePrivacy: () => { hideBalances = !hideBalances; patchLive(); }, onTab: navigate };
}
function overview(activeTab: "assets" | "activity" = "assets"): HTMLElement { return walletOverview(overviewProps(activeTab)); }
function pickerProps(mode: "deposit" | "send"): Parameters<typeof assetPicker>[0] {
  return { mode, networks: portfolioViews(), hidden: hideBalances, onSelect: ref => beginTransfer(mode, ref), onBack: () => navigate("assets") };
}
function chooseAssetView(mode: "deposit" | "send"): HTMLElement { return assetPicker(pickerProps(mode)); }
function allRecords(): PaymentRecord[] { return [...portfolio.values()].flatMap(view => view.records).sort((a, b) => b.created_at - a.created_at); }
function pendingView(): HTMLElement {
  const pending = (["assets", "activity"].includes(route) ? allRecords() : state().records).find(isActive);
  const slot = el("div", { id: "pending-slot" });
  if (pending) slot.append(el("div", { class: "active-payment" }, icon("activity"), el("div", {}, el("strong", {}, `${statusLabel(pending)} · ${netLabel(pending.prepared.request.chain_id)}`), el("p", {}, pending.phase === "prepared" ? "An unsigned transfer is saved. Continue or discard it." : "This transfer is saved and will be checked automatically.")), button("View", () => void openPayment(pending.intent_id), "text compact", "chevron")));
  return slot;
}
function replaceDisplay(id: string, next: HTMLElement): void {
  const previous = document.getElementById(id);
  if (!previous || previous.isEqualNode(next)) return;
  // Keep a focused disclosure or action stable until it loses focus. A following
  // update will bring the secondary display up to date without stealing focus.
  if (id !== "fee-panel" && previous.contains(document.activeElement) && document.activeElement !== document.body) return;
  const expanded = [...previous.querySelectorAll("details")].map(item => item.open);
  for (const [index, item] of [...next.querySelectorAll("details")].entries()) item.open = expanded[index] ?? false;
  previous.replaceWith(next);
}
function patchLive(): void {
  // Automatic reads never reconstruct a form, a QR code, a secret screen or the
  // portfolio's controls. Each chain only updates its own observations.
  if (!unlocked || !vault?.backup_confirmed) return;
  const nextConnection = connectionView();
  replaceDisplay(nextConnection.id, nextConnection);
  const overviewNode = document.getElementById("wallet-overview");
  if (overviewNode && (route === "assets" || route === "activity")) patchWalletOverview(overviewNode, overviewProps(route));
  const pickerNode = document.getElementById("asset-picker");
  if (pickerNode && (route === "choose-deposit" || route === "choose-send")) patchAssetPicker(pickerNode, pickerProps(route === "choose-deposit" ? "deposit" : "send"));
  const assetNode = document.getElementById("asset-overview");
  if (route === "asset" && assetNode) patchAssetOverview(assetNode, assetBalanceProps());
  replaceDisplay("pending-slot", pendingView());
  if (route === "activity" || route === "asset") {
    replaceDisplay("activity-body", activityBody(route === "asset" ? 3 : undefined, route === "asset" ? selectedAsset : undefined));
    replaceDisplay("activity-status", activityStatus());
  }
  const available = document.getElementById("available-balance");
  if (available) available.textContent = availableText();
  if (route === "transaction" && !busy) replaceDisplay("transaction-details", transactionView());
  if (route === "chain-transaction" && !busy) replaceDisplay("chain-details", chainTransactionView());
  if (busy) updateBusy();
  updateReviewButton();
}
function topbar(): HTMLElement {
  const active = route === "activity" || route === "transaction" || route === "retry" || route === "chain-transaction" ? "activity" : ["profile", "rename", "settings", "app-lock", "set-pin", "disable-lock", "reveal", "words"].includes(route) ? "profile" : "assets";
  return walletHeader({ unlocked, navigationEnabled: Boolean(vault?.backup_confirmed), active, ...displayNames, disabled: Boolean(busy), lockEnabled: lockStatus.mode !== "off", onNavigate: navigate, onLock: lock, onHome: () => navigate(unlocked ? "assets" : vault ? "unlock" : "welcome") });
}
function heading(title: string, subtitle: string, action?: HTMLElement): HTMLElement {
  return el("div", { class: "page-heading" }, el("div", {}, el("h1", {}, title), el("p", { class: "muted" }, subtitle)), action ?? null);
}
function welcome(): HTMLElement {
  return el("div", { class: "welcome" },
    el("div", { class: "welcome-copy" }, el("div", { class: "welcome-emblem" }, icon("wallet")), el("h1", {}, "Your crypto.", el("br"), "Your control."), el("p", { class: "lead" }, "Send and receive on Base and Ethereum. Your keys stay with you."),
      el("div", { class: "welcome-assets" }, ...(["USDC", "EURC", "ETH"] as Asset[]).map(symbol => el("span", {}, assetIcon(symbol), symbol)))),
    el("div", { class: "welcome-actions" }, button("Create a new wallet", () => navigate("create"), "primary wide"), button("Import an existing wallet", () => navigate("restore"), "secondary wide")),
    el("p", { class: "fine-print" }, "Your recovery words restore your wallet. Keep them private and offline."),
    el("p", { class: "fine-print" }, "Mainnet development release · Independent security audit pending."));
}
function passwordField(label: string, autocomplete: string): { node: HTMLElement; input: HTMLInputElement } {
  const element = input("password", "", { autocomplete, minlength: 12, maxlength: 1024, required: true, spellcheck: "false" });
  return { node: field(label, element), input: element };
}
function pinField(label: string): { node: HTMLElement; input: HTMLInputElement } {
  const element = input("password", "", { class: "pin-input", inputmode: "numeric", pattern: "[0-9]{6}", minlength: 6, maxlength: 6, autocomplete: "off", required: true, spellcheck: "false", "data-private": "true" });
  return { node: field(label, element, "Exactly 6 digits."), input: element };
}
function accessField(): { node: HTMLElement | null; input: HTMLInputElement | null } {
  if (lockStatus.mode === "off") return { node: null, input: null };
  return lockStatus.mode === "pin" ? pinField("6-digit app PIN") : passwordField("Existing wallet password", "current-password");
}
function lockLabel(): string {
  return lockStatus.mode === "off" ? "Off" : lockStatus.mode === "pin" ? "6-digit app PIN" : "Existing wallet password";
}
function takeInput(element: HTMLInputElement | null): string {
  const value = element?.value ?? "";
  if (element) element.value = "";
  return value;
}
async function operationCredential(expected: Vault, value: string, authorized: () => boolean): Promise<{
  secret: string; vault: Vault; status: WalletLockStatus; current: () => Promise<boolean>;
}> {
  const state = await journal.protectionState(expected);
  let secret = "";
  try {
    secret = await protection.credentials(state.vault, value, authorized);
    const current = async (): Promise<boolean> => {
      if (!authorized()) return false;
      await journal.assertProtectionCurrent(state.vault, state.protection);
      return authorized();
    };
    if (!await current()) throw new Error("Wallet operation was cancelled.");
    return { secret, vault: state.vault, status: state.protection
      ? { mode: state.protection.mode, retry_at: 0, failed_attempts: 0 }
      : { mode: "password", retry_at: 0, failed_attempts: 0 }, current };
  } finally { value = ""; secret = ""; }
}
function createRestore(restoring: boolean): HTMLElement {
  const mnemonic = el("textarea", { rows: 4, autocomplete: "off", autocapitalize: "none", spellcheck: "false", maxlength: 512, placeholder: "Enter your recovery words in order", required: restoring, "data-private": "true" });
  const acknowledged = input("checkbox", "", { required: true });
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault();
    if (!form.reportValidity()) return;
    let words = mnemonic.value; mnemonic.value = "";
    void run(restoring ? "Restoring your local wallet…" : "Creating your recovery words…", async authorized => {
      let createdWords = "";
      try {
        const created = restoring ? { vault: await protection.restore(words, authorized), mnemonic: "" } : await protection.create(authorized);
        createdWords = created.mnemonic;
        if (!authorized()) return;
        // Vault and its device protection commit together before words are shown.
        vault = created.vault; lockStatus = await protection.status(vault);
        if (!authorized()) return;
        unlocked = true; lastInteraction = Date.now();
        if (navigator.storage?.persist) void navigator.storage.persist().catch(() => false);
        if (restoring) { route = "assets"; hydrateAndRefresh(authorized); }
        else { phrase = createdWords; route = "backup"; }
      } finally { words = ""; createdWords = ""; }
    });
  } },
    restoring ? field("Recovery words", mnemonic, "English BIP39, 12–24 words. Uses the first Ethereum account and an empty BIP39 passphrase.") : null,
    el("p", { class: "field-help" }, "Your wallet is encrypted on this device. App lock is optional; you can add a 6-digit PIN in Profile → Settings."),
    el("label", { class: "check-label" }, acknowledged, el("span", {}, "I understand that my recovery words are needed if I lose this device or its browser data.")),
    el("button", { type: "submit", class: "button primary wide" }, restoring ? "Import wallet" : "Create wallet", icon("arrow")));
  return el("div", { class: "narrow" }, button("Back", () => navigate("welcome"), "text", "back"), heading(restoring ? "Import your wallet" : "Create your wallet", "No app passcode is required."), card(restoring ? "Bring your account with you" : "Save your recovery words", restoring ? "Your existing words restore the same account on Base and Ethereum." : "Next, save your 24 recovery words and confirm your backup.", form));
}
async function openSavedWallet(userInput: string, authorized: () => boolean): Promise<void> {
  let credential: Awaited<ReturnType<typeof operationCredential>> | null = null;
  let restoredWords = "";
  try {
    const saved = await journal.vault();
    if (!saved) throw new Error("The saved wallet is missing. No new wallet has been created.");
    credential = await operationCredential(saved, userInput, authorized);
    if (!saved.backup_confirmed) {
      restoredWords = await core.reveal(saved, credential.secret);
    }
    if (!await credential.current()) { phrase = ""; return; }
    phrase = restoredWords;
    vault = saved; lockStatus = credential.status; unlocked = true; lastInteraction = Date.now();
    route = saved.backup_confirmed ? "assets" : "backup";
    if (saved.backup_confirmed) hydrateAndRefresh(authorized);
  } finally { if (credential) credential.secret = ""; userInput = ""; restoredWords = ""; }
}
function unlockView(): HTMLElement {
  const credential = accessField();
  const pendingBackup = Boolean(vault && !vault.backup_confirmed);
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (!form.reportValidity() || !vault) return;
    let value = takeInput(credential.input);
    void run(pendingBackup ? "Resuming your saved recovery backup…" : "Opening your wallet…", async authorized => {
      try { await openSavedWallet(value, authorized); } finally { value = ""; }
    });
  } }, credential.node,
    el("button", { type: "submit", class: "button primary wide" }, icon(pendingBackup ? "key" : "lock"), pendingBackup ? "Continue backup" : lockStatus.mode === "off" ? "Open wallet" : "Unlock wallet"));
  const help = lockStatus.mode === "password" ? "Use the password you created in the earlier version, at least 12 characters. After opening, you can change to a 6-digit PIN or turn app lock off in Profile → Settings."
    : lockStatus.mode === "pin" ? "Enter the 6-digit PIN set for this app. It is separate from your device passcode."
    : "App lock is off. Your encrypted wallet remains on this device.";
  return el("div", { class: "unlock-layout narrow" }, el("div", { class: "hero-symbol" }, icon(pendingBackup ? "key" : "lock")), heading(pendingBackup ? "Finish your wallet backup" : "Welcome back", pendingBackup ? "Your wallet was saved. Continue with the same recovery words." : "Open the wallet saved on this device."),
    card(pendingBackup ? "Resume setup" : "Your wallet", help, form, el("p", { class: "fine-print" }, "Your recovery words restore this account if you lose access. No server can reset a forgotten app PIN or wallet password.")));
}
function wordGrid(): HTMLElement {
  return el("ol", { class: "word-grid", "data-private": "true" }, ...phrase.split(" ").map((word, index) => el("li", {}, el("span", {}, String(index + 1).padStart(2, "0")), el("strong", {}, word))));
}
async function copyRecoveryWords(): Promise<void> {
  if (!phrase || !unlocked || !["backup", "words"].includes(route)) return;
  const session = epoch;
  try {
    await navigator.clipboard.writeText(phrase);
    if (session !== epoch) return;
    notice = "Recovery words copied. Clear your clipboard after saving them privately."; renderMessages();
  } catch {
    if (session !== epoch) return;
    error = "Clipboard access was unavailable. You can select the words to copy them, or take a screenshot."; renderMessages();
  }
}
function backupView(revealing = false): HTMLElement {
  if (!phrase) return el("div", { class: "narrow" }, info("Your words are hidden. Open the saved wallet to continue."), button("Continue", () => { lock(); }));
  const count = phrase.split(" ").length;
  return el("div", { class: "narrow wide-narrow" }, heading(revealing ? "Your recovery words" : "Save your recovery words", `Keep all ${count} words in this exact order.`),
    card("Your wallet’s recovery backup", "These words restore this same account in a compatible wallet.", wordGrid(),
      el("div", { class: "recovery-actions" }, button(`Copy ${count} words`, () => void copyRecoveryWords(), "secondary wide", "copy")),
      el("p", { class: "recovery-caption" }, "Screenshots are allowed. Anyone with a copy can control your assets. Clipboard history, photo backups and cloud sync may store that copy."),
      revealing ? button("Hide words", () => { phrase = ""; navigate("settings"); }, "primary wide", "check") : button("I saved my recovery words", () => {
        const indexes = new Set<number>(); const length = phrase.split(" ").length;
        while (indexes.size < 3) indexes.add(crypto.getRandomValues(new Uint32Array(1))[0] % length);
        challenge = [...indexes].sort((a, b) => a - b); navigate("challenge");
      }, "primary wide", "arrow")));
}
function challengeView(): HTMLElement {
  const inputs = challenge.map(index => ({ index, input: input("text", "", { autocomplete: "off", autocapitalize: "none", spellcheck: "false", required: true, maxlength: 20 }) }));
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (!form.reportValidity() || !vault) return;
    const words = phrase.split(" ");
    if (inputs.some(item => item.input.value.trim().toLowerCase() !== words[item.index])) { error = "One or more words do not match. Check the order in your saved backup."; render(); return; }
    for (const item of inputs) item.input.value = "";
    void run("Saving backup confirmation…", async authorized => {
      vault = await journal.confirmBackup(vault!); phrase = ""; challenge = [];
      if (!authorized()) return;
      route = "assets"; hydrateAndRefresh(authorized);
    });
  } }, ...inputs.map(item => field(`Word ${item.index + 1}`, item.input)), el("button", { type: "submit", class: "button primary wide" }, "Confirm backup", icon("check")));
  return el("div", { class: "narrow" }, heading("Check your backup", "Use your saved copy to confirm these three words."), card("Confirm the word order", "This check helps catch a missing or misplaced recovery word.", form, button("Show words again", () => navigate("backup"), "text wide")));
}
function assetsView(): HTMLElement {
  return el("div", { class: "dashboard" }, overview(), connectionView(), pendingView());
}
function openReceive(asset: Asset, back: Route): void {
  receiveAsset = asset; receiveBack = back; navigate("receive");
}
function assetBalanceProps(): Parameters<typeof assetOverview>[0] {
  const { snapshot, refreshingBalance, connectionState } = state();
  return { chainId: chain, symbol: selectedAsset,
    amount: snapshot ? formatUnits(snapshot.balances[selectedAsset], networkAsset(network(), selectedAsset).decimals) : null,
    loading: !snapshot && (refreshingBalance || connectionState === "idle" || connectionState === "loading"), stale: connectionState === "stale", hidden: hideBalances,
    onSend: () => openSend(selectedAsset), onReceive: () => openReceive(selectedAsset, "asset") };
}
function assetBalanceView(): HTMLElement { return assetOverview(assetBalanceProps()); }
function assetView(): HTMLElement {
  const spec = networkAsset(network(), selectedAsset);
  return el("div", { class: "asset-page narrow" }, button("Back", () => navigate("assets"), "text", "back"), heading(displayName(selectedAsset), selectedAsset), assetBalanceView(), connectionView(), pendingView(),
    el("section", { class: "asset-metadata" }, detail("Network", el("span", { class: "network-label-pill" }, chainMark(chain), netLabel())),
      spec.contract ? el("details", { class: "fee-details" }, el("summary", {}, "Token details"), detail("Contract", spec.contract, true), detail("Decimals", String(spec.decimals)), el("a", { class: "external-link", href: `${network().explorer_url}/token/${spec.contract}`, target: "_blank", rel: "noopener noreferrer" }, "View token on explorer", icon("external"))) : el("p", { class: "fine-print" }, "ETH pays network fees on this network.")),
    el("section", { class: "activity-card" }, el("div", { class: "section-title" }, el("h2", {}, "Recent activity"), button("View all", () => navigate("activity"), "text compact", "chevron")), activityStatus(), activityBody(3, selectedAsset)));
}
function receiveView(): HTMLElement {
  const select = button(receiveAsset, () => navigate("choose-deposit"), "text");
  select.append(icon("down"));
  select.setAttribute("aria-label", "Change deposit asset and network");
  const canvas = el("canvas", { class: "qr", "aria-label": `Receive ${receiveAsset} on ${netLabel()} QR code`, role: "img" });
  void QRCode.toCanvas(canvas, receiveUri(network(), vault!.address, receiveAsset), { width: 224, margin: 3, errorCorrectionLevel: "M", color: { dark: "#0B0D10", light: "#ffffff" } }).catch(() => { canvas.replaceWith(info("QR unavailable. Use the address below.")); });
  const spec = networkAsset(network(), receiveAsset);
  return el("div", { class: "narrow receive-page" }, button("Back", () => navigate(receiveBack), "text", "back"),
    heading("Deposit", `${receiveAsset} on ${netLabel()}`),
    el("section", { class: "card receive-card" }, el("div", { class: "receive-select" }, assetIcon(receiveAsset), select, el("span", { class: "network-label-pill" }, chainMark(chain), netLabel())),
      el("div", { class: "qr-wrap" }, canvas), el("p", { class: "field-label" }, "Your wallet address"), el("p", { class: "full-address mono", "data-testid": "receive-address" }, vault!.address),
      button("Copy address", () => void copyAddress(chain), "primary wide", "copy"),
      el("p", { class: "receive-note" }, "Only send ", el("strong", {}, `${receiveAsset} on ${netLabel()}`), " to this address. Select the same network in the sending wallet."),
      spec.contract ? el("details", { class: "fee-details" }, el("summary", {}, `${receiveAsset} token details`), detail("Contract", spec.contract, true)) : null),
    connectionView(), el("p", { class: "watching-note" }, icon("activity"), "Checking for incoming transfers automatically."));
}
function draftFingerprint(): string {
  return JSON.stringify([chain, vault?.address, sendDraft.asset, sendDraft.to.trim(), sendDraft.amount.trim()]);
}
function validDraft(): boolean {
  try { validateAddress(sendDraft.to.trim()); return units(sendDraft.amount.trim(), networkAsset(network(), sendDraft.asset).decimals) > 0n; }
  catch { return false; }
}
function currentPreview(): FeePreview | null {
  return preview && previewKey === draftFingerprint() && preview.chain_id === chain && preview.asset === sendDraft.asset && Math.floor(Date.now() / 1000) - preview.estimated_at <= 90 ? preview : null;
}
function availableText(): string {
  const { snapshot, connectionState } = state();
  return snapshot ? `${formatUnits(snapshot.balances[sendDraft.asset], networkAsset(network(), sendDraft.asset).decimals)} ${sendDraft.asset} available${connectionState === "stale" ? " · last saved balance" : ` on ${netLabel()}`}` : "Balance unavailable — a fee can still be estimated.";
}
function draftChanged(): void {
  resetPreview(); error = ""; renderMessages();
  const available = document.getElementById("available-balance"); if (available) available.textContent = availableText();
  patchFee(); patchDraftFeedback();
  if (validDraft()) scheduleEstimate();
}
function patchDraftFeedback(): void {
  let recipientProblem = "", amountProblem = "";
  if (sendDraft.to.trim()) {
    try { validateAddress(sendDraft.to.trim()); }
    catch { recipientProblem = "Enter a complete 0x address (42 characters). The zero address is not allowed."; }
  }
  if (sendDraft.amount.trim()) {
    try { if (units(sendDraft.amount.trim(), networkAsset(network(), sendDraft.asset).decimals) === 0n) amountProblem = "Enter an amount greater than zero."; }
    catch (failure) { amountProblem = failure instanceof Error ? failure.message : "Enter a valid amount."; }
  }
  for (const [id, message] of [["recipient", recipientProblem], ["amount", amountProblem]]) {
    const feedback = document.getElementById(`${id}-error`);
    if (feedback) feedback.textContent = message;
    const control = document.getElementById(`send-${id}`);
    if (control) control.setAttribute("aria-invalid", String(Boolean(message)));
  }
}
function scheduleEstimate(): void {
  if (feeTimer) clearTimeout(feeTimer);
  feeTimer = setTimeout(() => { feeTimer = null; if (route === "send" && unlocked && !busy && validDraft()) void estimateFee(); }, 800);
}
async function estimateFee(): Promise<void> {
  if (!vault || !unlocked || busy || !validDraft()) return;
  feeAbort?.abort();
  const abort = new AbortController(); feeAbort = abort;
  const token = ++feeGeneration, session = epoch, fingerprint = draftFingerprint();
  const valid = () => token === feeGeneration && session === epoch && fingerprint === draftFingerprint() && unlocked;
  feeLoading = true; feeError = ""; patchFee();
  try {
    const result = await rpc().preview({ from: vault.address, to: sendDraft.to.trim(), asset: sendDraft.asset, amount: sendDraft.amount.trim() }, abort.signal);
    if (!valid()) return;
    preview = result; previewKey = fingerprint;
  } catch (failure) {
    if (valid() && !abort.signal.aborted) { preview = null; previewKey = ""; feeError = failure instanceof Error ? failure.message : "The network fee could not be estimated."; }
  } finally {
    if (valid()) { feeLoading = false; feeAbort = null; patchFee(); }
  }
}
function feeView(): HTMLElement {
  const quote = currentPreview();
  const eth = (value: string | undefined) => value === undefined ? null : formatUnits(value, 18);
  return feePanel({ state: feeLoading ? "loading" : quote ? "ready" : feeError ? "error" : "empty",
    estimatedFeeEth: eth(quote?.estimated_total_fee_wei), budgetEth: eth(quote?.fee_reserve_wei), executionFeeEth: eth(quote?.estimated_execution_fee_wei),
    l1FeeEth: eth(quote?.l1_fee_estimate_wei), operatorFeeEth: eth(quote?.operator_fee_estimate_wei), isBase: chain === 8453,
    error: feeError, warnings: quote?.warnings ?? [], shortages: quote?.shortages ?? [],
    source: quote ? `${quote.estimate_source === "simulation" ? "Estimated from a transfer simulation" : "Conservative estimate; final simulation occurs before review"} · ${new Date(quote.estimated_at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : null,
    onEstimate: () => void estimateFee() });
}
function patchFee(): void {
  replaceDisplay("fee-panel", feeView());
  const funding = document.getElementById("fee-funding");
  if (funding) {
    const shortage = currentPreview()?.shortages ?? [];
    const assets = new Set<Asset>();
    if (shortage.some(text => text.startsWith("ETH needed") || text.startsWith("Insufficient ETH"))) assets.add("ETH");
    if (shortage.some(text => text.startsWith(`Insufficient ${sendDraft.asset}`))) assets.add(sendDraft.asset);
    funding.replaceChildren(...[...assets].map(asset => button(`Deposit ${asset} on ${netLabel()}`, () => { receiveAsset = asset; receiveBack = "send"; navigate("receive"); }, "text compact", "receive")));
  }
  updateReviewButton();
}
function updateReviewButton(): void {
  const next = document.getElementById("review-transfer") as HTMLButtonElement | null;
  if (next) next.disabled = Boolean(busy || feeLoading || !validDraft() || !currentPreview() || currentPreview()!.shortages.length || state().records.some(isActive));
}
function sendView(): HTMLElement {
  const active = state().records.find(isActive);
  if (active) return el("div", { class: "narrow" }, button("Back", () => navigate("assets"), "text", "back"), heading("Continue your transfer", "An existing transfer needs your attention."), card(statusLabel(active), `${active.prepared.request.amount} ${active.prepared.request.asset}`, el("p", { class: "muted" }, active.phase === "prepared" ? "This saved draft has not been signed. Review it or discard it before creating another transfer." : "The signed payment is saved. Check its status before making another transfer on this network."), button("Open transfer", () => void openPayment(active.intent_id), "primary wide", "arrow")));
  const recipient = input("text", sendDraft.to, { id: "send-recipient", class: "mono", placeholder: "Recipient’s 0x address", autocomplete: "off", autocapitalize: "none", spellcheck: "false", required: true, maxlength: 42, "aria-describedby": "recipient-error", oninput: (event: Event) => { sendDraft.to = (event.target as HTMLInputElement).value; draftChanged(); } });
  const amount = input("text", sendDraft.amount, { id: "send-amount", class: "amount-input", placeholder: "0.00", inputmode: "decimal", autocomplete: "off", required: true, maxlength: 100, "aria-describedby": "amount-error available-balance", oninput: (event: Event) => { sendDraft.amount = (event.target as HTMLInputElement).value; draftChanged(); } });
  const badge = el("span", { id: "send-asset-badge" }, assetIcon(sendDraft.asset));
  const asset = button(sendDraft.asset, () => navigate("choose-send"), "text compact");
  asset.append(icon("down"));
  asset.id = "send-asset";
  asset.setAttribute("aria-label", "Change asset and network");
  const recipientField = field("Recipient address", recipient, `The recipient must support ${netLabel()}. Check the full address.`);
  recipientField.append(el("p", { id: "recipient-error", class: "field-error", "aria-live": "polite" }));
  const amountField = el("div", { class: "amount-entry" }, el("div", { class: "amount-entry-head" }, el("label", { for: "send-amount" }, "Amount"), el("div", { class: "asset-picker" }, badge, asset)), amount,
    el("span", { id: "available-balance", class: "available-balance" }, availableText()), el("p", { id: "amount-error", class: "field-error", "aria-live": "polite" }));
  const submit = el("button", { id: "review-transfer", type: "submit", class: "button primary wide", disabled: !currentPreview() || Boolean(currentPreview()?.shortages.length) }, "Review transfer", icon("arrow"));
  const form = el("form", { id: "send-form", class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (!form.reportValidity() || !currentPreview()) return;
    const displayed = currentPreview()!;
    if (displayed.shortages.length) return;
    void run("Checking your transfer…", async authorized => {
      const prepared = await engine().prepare(vault!, sendDraft.to.trim(), sendDraft.asset, sendDraft.amount.trim());
      if (!authorized()) return;
      reviewed = prepared; reviewedPreview = displayed; route = "review";
      state().records = await journal.list(chain, vault!.address);
    });
  } }, el("div", { class: "transfer-network" }, chainMark(chain), el("strong", {}, `${netLabel()} mainnet`), el("span", {}, "Fees paid in ETH")),
    recipientField, amountField, feeView(), el("div", { id: "fee-funding" }), submit,
    el("p", { class: "form-caption" }, "Review the recipient and fee before you confirm."));
  return el("div", { class: "send-layout" }, el("div", { class: "narrow send-column" }, button("Back", () => navigate("assets"), "text", "back"), heading("Send", `Transfer assets on ${netLabel()}.`), el("section", { class: "card send-card" }, form), connectionView()));
}
function paymentReview(prepared: PreparedPayment): HTMLElement {
  const p = prepared.request;
  const spec = networkAsset(network(), p.asset);
  const estimateMatches = reviewedPreview && reviewedPreview.chain_id === p.chain_id && reviewedPreview.asset === p.asset && reviewedPreview.gas_limit === p.gas_limit && reviewedPreview.max_fee_per_gas === p.max_fee_per_gas && reviewedPreview.max_priority_fee_per_gas === p.max_priority_fee_per_gas && reviewedPreview.l1_fee_estimate_wei === p.l1_fee_estimate_wei && reviewedPreview.operator_fee_estimate_wei === p.operator_fee_estimate_wei;
  return el("div", { class: "review-summary" }, el("div", { class: "review-amount" }, assetIcon(p.asset), el("strong", {}, p.amount), el("span", {}, p.asset)),
    el("div", { class: "review-network" }, chainMark(chain), `${netLabel()} mainnet`),
    detail("To", p.to, true),
    estimateMatches ? detail("Estimated network fee", `${formatUnits(reviewedPreview!.estimated_total_fee_wei, 18)} ETH`) : null,
    detail("Network fee budget", `${formatUnits(prepared.fee_reserve_wei, 18)} ETH`),
    el("p", { class: "fee-help" }, "This budget includes a buffer. The actual fee may be lower."),
    el("details", { class: "fee-details" }, el("summary", {}, "Transaction & fee details"), el("div", {}, detail("From", p.from, true), spec.contract ? detail(`${p.asset} contract`, spec.contract, true) : null,
      detail("Execution fee cap", `${formatUnits(prepared.max_fee_wei, 18)} ETH`), chain === 8453 ? detail("L1 data estimate", `${formatUnits(p.l1_fee_estimate_wei, 18)} ETH`) : null, chain === 8453 ? detail("Operator estimate", `${formatUnits(p.operator_fee_estimate_wei, 18)} ETH`) : null,
      detail("Nonce", String(p.nonce)), detail("Review expires", new Date(prepared.expires_at * 1000).toLocaleTimeString()),
      el("p", { class: "fine-print" }, chain === 8453 ? "Base’s L1 and operator fees can vary. The buffer is a budget, not a hard cap on those charges. Only the execution fee is capped in the signed transaction." : "A reverted transaction still pays for gas."))));
}
async function editReview(prepared: PreparedPayment): Promise<void> {
  await run("Returning to your transfer…", async authorized => {
    await journal.cancel(prepared.request.intent_id);
    if (!authorized()) return;
    sendDraft = { to: prepared.request.to, amount: prepared.request.amount, asset: prepared.request.asset };
    reviewed = null; reviewedPreview = null; route = "send";
    state().records = await journal.list(chain, vault!.address);
    scheduleEstimate();
  });
}
function reviewView(): HTMLElement {
  if (!reviewed) return sendView();
  const frozen = reviewed;
  const credential = accessField();
  const checked = input("checkbox", "", { required: true });
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (!form.reportValidity()) return;
    let value = takeInput(credential.input);
    let access: Awaited<ReturnType<typeof operationCredential>> | null = null;
    void run("Checking, signing and submitting…", async authorized => {
      try {
        access = await operationCredential(vault!, value, authorized);
        if (!authorized() || !unlocked) return;
        const permit = access;
        if (frozen.request.chain_id !== chain) throw new Error("The selected network changed. Review the transfer again.");
        const result = await engine(frozen.request.chain_id).signAndSubmit(permit.vault, permit.secret, frozen, async () => unlocked && chain === frozen.request.chain_id && await permit.current() && unlocked);
        if (!authorized()) return;
        currentPayment = result.record; reviewed = null; reviewedPreview = null; route = "transaction";
        notice = result.acknowledged ? "Transfer submitted. We’ll check for confirmation automatically." : "Submission could not be confirmed. Your transaction is saved; check its status before retrying.";
        state().records = await journal.list(chain, vault!.address);
        startRefreshSoon();
      } finally { if (access) access.secret = ""; value = ""; }
    });
  } }, el("label", { class: "check-label" }, checked, el("span", {}, `I checked the full recipient address and the ${netLabel()} network.`)), credential.node, el("button", { type: "submit", class: "button primary wide" }, icon("lock"), `Confirm and send ${frozen.request.asset}`));
  return el("div", { class: "narrow review-page" }, button("Edit transfer", () => void editReview(frozen), "text", "back"), heading("Confirm transfer", "Check the recipient and network before sending."), el("section", { class: "card" }, paymentReview(frozen), form));
}
function statusLabel(payment: PaymentRecord): string {
  const values = {
    prepared: "Unsigned draft", signed: "Submission unconfirmed", unknown: "Submission unknown", pending: "Submitted · not confirmed",
    included: payment.execution === "failed" ? "Reverted · awaiting finality" : "Included · awaiting finality",
    finalized: payment.execution === "failed" ? "Reverted · gas charged" : "Finalized transfer",
    unresolved: "Needs investigation", cancelled: "Discarded unsigned",
  };
  return values[payment.phase];
}
function statusTone(payment: PaymentRecord): string { return payment.phase === "finalized" && payment.execution === "success" ? "success" : payment.phase === "unresolved" || payment.execution === "failed" ? "warning" : "neutral"; }
async function openPayment(id: string): Promise<void> {
  await run("Reading saved payment…", async authorized => {
    const payment = await journal.get(id);
    if (!authorized()) return;
    if (payment.prepared.request.from.toLowerCase() !== vault?.address.toLowerCase()) throw new Error("This transfer does not belong to the open account.");
    selectOperationChain(payment.prepared.request.chain_id);
    currentPayment = payment; reviewed = payment.phase === "prepared" ? payment.prepared : null; route = "transaction";
  });
}
function displayActivities(): ActivityItemView[] {
  const records = allRecords();
  const items: ActivityItemView[] = records.map(payment => {
    const notSent = payment.phase === "prepared" || payment.phase === "cancelled" || payment.execution === "failed";
    return {
      id: payment.intent_id, chainId: payment.prepared.request.chain_id, direction: "outgoing", symbol: payment.prepared.request.asset, amount: payment.prepared.request.amount,
      counterparty: payment.prepared.request.to, status: statusLabel(payment), statusTone: statusTone(payment) as ActivityItemView["statusTone"], timestamp: payment.created_at,
      title: payment.phase === "prepared" ? "Transfer draft" : payment.phase === "cancelled" ? "Discarded draft" : payment.execution === "failed" ? "Transfer reverted" : payment.execution === "success" ? `Sent ${payment.prepared.request.asset}` : `Sending ${payment.prepared.request.asset}`,
      amountPrefix: notSent ? "" : "−", onOpen: () => void openPayment(payment.intent_id),
    };
  });
  const local = new Set(records.filter(item => item.signed).map(item => `${item.prepared.request.chain_id}:${item.signed!.hash.toLowerCase()}:${item.prepared.request.asset}`));
  for (const item of [...portfolio.values()].flatMap(view => view.chainItems)) {
    if (item.direction !== "received" && local.has(`${item.chain_id}:${item.transaction_hash.toLowerCase()}:${item.asset}`)) continue;
    const incoming = item.direction === "received", self = item.direction === "self", removed = item.inclusion === "reorged";
    items.push({ id: item.id, chainId: item.chain_id, direction: incoming ? "incoming" : "outgoing", symbol: item.asset, amount: item.amount,
      counterparty: incoming ? item.from : item.to, status: removed ? "Rechecking after reorg" : item.inclusion === "finalized" ? "Confirmed" : "Included · confirming",
      statusTone: removed ? "warning" : item.inclusion === "finalized" ? "success" : "neutral", timestamp: item.timestamp,
      title: removed ? "Reorganized transfer" : self ? "Self transfer" : incoming ? `Received ${item.asset}` : `Sent ${item.asset}`,
      amountPrefix: removed || self ? "" : incoming ? "+" : "−", onOpen: () => { if (busy) return; selectOperationChain(item.chain_id); currentActivity = state(item.chain_id).chainItems.find(saved => saved.id === item.id) ?? item; navigate("chain-transaction"); },
    });
  }
  return items.sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0));
}
function activityStates(): [ChainId, NetworkState][] {
  return route === "asset" ? [[chain, state()]] : ([8453, 1] as ChainId[]).map(id => [id, state(id)]);
}
function activityBody(limit?: number, asset?: Asset): HTMLElement {
  const items = displayActivities().filter(item => !asset || (item.symbol === asset && item.chainId === chain));
  const selected = limit ? items.slice(0, limit) : items;
  if (selected.length) return el("div", { id: "activity-body" }, activityList(selected, hideBalances));
  const states = activityStates().map(([,view]) => view);
  if (states.some(view => view.refreshingActivity)) return el("div", { id: "activity-body", class: "activity-loading", "aria-label": "Loading recent transactions" }, ...[1, 2, 3].map(() => el("div", { class: "activity-skeleton" }, el("span"), el("span"), el("span"))));
  const unavailable = states.some(view => view.activityError);
  return el("div", { id: "activity-body", class: "empty-state" }, icon("activity"), el("h3", {}, unavailable ? "Activity unavailable" : "No recent activity"), el("p", {}, unavailable ? "Your balances are checked separately. You can look up a transfer below." : "Verified deposits and sends appear here."));
}
function activityStatus(): HTMLElement {
  const states = activityStates(), loading = states.some(([,view]) => view.refreshingActivity), failed = states.some(([,view]) => view.activityError);
  const label = loading ? "Updating activity…" : failed ? "Some history could not be refreshed" : "Recent activity";
  return el("div", { id: "activity-status", class: "activity-status" }, el("span", {}, label),
    el("details", { class: "coverage-details" }, el("summary", {}, "Details"), el("div", {}, el("p", {}, "Recent verified activity. Saved transfers and transaction lookups are retained; this is not a complete account index."),
      ...states.map(([id,view]) => el("div", {}, el("strong", {}, netLabel(id)),
        view.activityResult?.from_block !== null && view.activityResult?.from_block !== undefined ? el("p", {}, `Blocks ${view.activityResult.from_block.toLocaleString("en-US")}–${view.activityResult.to_block?.toLocaleString("en-US") ?? "latest"}`) : null,
        view.activityError ? el("p", { class: "connection-error" }, view.activityError) : null, ...(view.activityResult?.warnings ?? []).map(text => el("p", {}, text)))))));
}
function patchLookup(): void {
  const button = document.getElementById("lookup-button") as HTMLButtonElement | null;
  if (button) { button.disabled = lookupRunning || Boolean(lookupQueuedHash) || !/^0x[0-9a-fA-F]{64}$/.test(lookupHash.trim()); button.textContent = lookupRunning ? "Checking…" : lookupQueuedHash ? "Queued…" : "Find transaction"; }
  const result = document.getElementById("lookup-result");
  if (result) result.textContent = lookupMessage;
  const selector = document.getElementById("lookup-network") as HTMLSelectElement | null;
  if (selector) selector.disabled = lookupRunning;
}
async function lookupTransaction(): Promise<void> {
  if (!vault || !unlocked || busy || lookupRunning || !/^0x[0-9a-fA-F]{64}$/.test(lookupHash.trim())) return;
  if (state(lookupChain).refreshingActivity) {
    lookupQueuedHash = lookupHash.trim();
    lookupMessage = "Finishing the current activity refresh. Your transaction will be checked next.";
    patchLookup(); return;
  }
  const session = epoch, selected = lookupChain, searched = lookupHash.trim(), owner = vault.address;
  const authorized = () => session === epoch && selected === lookupChain && unlocked;
  const sameSearch = () => lookupHash.trim().toLowerCase() === searched.toLowerCase();
  lookupQueuedHash = null; lookupRunning = true; lookupMessage = ""; patchLookup();
  try {
    const result: ActivityResult = await engine(selected).lookupTransaction(owner, searched);
    if (!authorized()) return;
    mergeActivity(result.items, selected);
    if (!sameSearch()) return;
    const found = result.items.filter(item => item.transaction_hash.toLowerCase() === searched.toLowerCase() && item.inclusion !== "reorged");
    const preserved = found.length ? " Earlier verified activity has been preserved." : "";
    switch (result.lookup_status) {
      case "received": case "sent": case "self":
        if (found.length) {
          lookupMessage = `Verified ${found.length} ${found.length === 1 ? "transfer" : "transfers"} on ${netLabel(selected)}. Your activity is updated.`;
          void refreshBalances(selected);
        } else lookupMessage = "No verified supported-asset transfer was returned for this wallet.";
        break;
      case "pending":
        lookupMessage = `The provider reports this transaction as pending. No new confirmed transfer was recorded.${preserved}`; break;
      case "not_found":
        lookupMessage = `The provider could not find this transaction on ${netLabel(selected)}. Check the hash and network.${preserved}`; break;
      case "unrelated":
        lookupMessage = `No supported-asset transfer for this wallet was found in this transaction.${preserved}`; break;
      case "failed":
        lookupMessage = `This transaction reverted. It did not complete the transfer; the sender may still have paid gas.${preserved}`; break;
      default:
        lookupMessage = `${result.error ?? "The transaction could not be verified from the network right now. Try again shortly."}${preserved}`;
    }
  } catch (failure) {
    if (authorized() && sameSearch()) lookupMessage = failure instanceof Error ? failure.message : "This transaction could not be checked.";
  } finally {
    if (authorized()) { lookupRunning = false; patchLive(); patchLookup(); }
  }
}
function activityView(): HTMLElement {
  const search = input("text", lookupHash, { id: "transaction-lookup", class: "mono", placeholder: "Transaction hash · 0x…", autocomplete: "off", autocapitalize: "none", spellcheck: "false", maxlength: 66, "aria-label": "Transaction hash", oninput: (event: Event) => { lookupHash = (event.target as HTMLInputElement).value; lookupQueuedHash = null; lookupMessage = ""; patchLookup(); } });
  const selector = el("select", { id: "lookup-network", "aria-label": "Transaction network", disabled: lookupRunning, onchange: (event: Event) => {
    const selected = Number((event.target as HTMLSelectElement).value);
    if (selected !== 1 && selected !== 8453) return;
    lookupChain = selected; lookupQueuedHash = null; lookupMessage = ""; patchLookup();
  } }, ...([8453, 1] as ChainId[]).map(id => el("option", { value: String(id), selected: id === lookupChain }, netLabel(id))));
  const lookup = el("form", { class: "lookup-form", onsubmit: (event: Event) => { event.preventDefault(); void lookupTransaction(); } }, search, el("button", { id: "lookup-button", type: "submit", class: "button secondary", disabled: lookupRunning || Boolean(lookupQueuedHash) || !/^0x[0-9a-fA-F]{64}$/.test(lookupHash.trim()) }, lookupRunning ? "Checking…" : lookupQueuedHash ? "Queued…" : "Find transaction"));
  return el("div", { class: "activity-page" }, overview("activity"), pendingView(),
    el("section", { class: "activity-card" }, activityStatus(), activityBody()), connectionView(),
    el("details", { class: "lookup-card" }, el("summary", {}, icon("history"), "Find a transfer", icon("down")), el("p", { class: "muted" }, "Choose the sending network and paste the transaction hash."), selector, lookup, el("p", { id: "lookup-result", class: "lookup-result", "aria-live": "polite" }, lookupMessage)),
    el("div", { class: "explorer-links" }, ...([8453, 1] as ChainId[]).map(id => el("a", { class: "external-link", href: `${network(id).explorer_url}/address/${vault!.address}`, target: "_blank", rel: "noopener noreferrer" }, `${netLabel(id)} explorer`, icon("external")))));
}
function chainTransactionView(): HTMLElement {
  const item = currentActivity;
  if (!item) return activityView();
  const title = item.inclusion === "reorged" ? "Transfer under review" : item.direction === "received" ? `Received ${item.asset}` : item.direction === "self" ? "Self transfer" : `Sent ${item.asset}`;
  return el("div", { id: "chain-details", class: "narrow" }, button("Back to activity", () => navigate("activity"), "text", "back"), heading(title, `${netLabel()} mainnet`),
    el("section", { class: "card" }, el("div", { class: "review-amount" }, assetIcon(item.asset), el("strong", {}, item.amount), el("span", {}, item.asset)),
      pill(item.inclusion === "finalized" ? "Confirmed" : item.inclusion === "reorged" ? "Reorganized · rechecking" : "Included · confirming", item.inclusion === "finalized" ? "success" : item.inclusion === "reorged" ? "warning" : "neutral"),
      el("div", { class: "details-list" }, detail("From", item.from, true), detail("To", item.to, true), detail("Transaction hash", item.transaction_hash, true), detail(item.inclusion === "reorged" ? "Previous block" : "Block", item.block_number.toLocaleString("en-US")), detail("Time", new Date(item.timestamp * 1000).toLocaleString()), item.asset_contract ? detail("Token contract", item.asset_contract, true) : null),
      el("a", { class: "button secondary wide", href: `${network().explorer_url}/tx/${item.transaction_hash}`, target: "_blank", rel: "noopener noreferrer" }, "Open transaction explorer", icon("external"))), connectionView());
}
function transactionView(): HTMLElement {
  const p = currentPayment;
  if (!p) return activityView();
  const refreshPayment = () => void run("Reconciling with the network…", async authorized => {
    const result = await engine().reconcile(p.intent_id);
    if (authorized()) { currentPayment = result; state().records = await journal.list(chain, vault!.address); }
  });
  const actions: HTMLElement[] = [];
  if (p.phase === "prepared") {
    actions.push(button("Review and sign", () => { reviewed = p.prepared; navigate("review"); }, "primary wide", "arrow"));
    actions.push(button("Discard unsigned draft", () => void run("Discarding unsigned draft…", async authorized => { const result = await journal.cancel(p.intent_id); if (authorized()) { currentPayment = result; state().records = await journal.list(chain, vault!.address); } }), "text wide"));
  } else if (p.signed) {
    actions.push(button("Check status", refreshPayment, "primary wide", "refresh"));
    if (["signed", "unknown", "pending"].includes(p.phase)) actions.push(button("Retry the same signed transaction", () => navigate("retry"), "secondary wide"));
    actions.push(el("a", { class: "external-link", href: `${network().explorer_url}/tx/${p.signed.hash}`, target: "_blank", rel: "noopener noreferrer" }, "Open transaction explorer", icon("external")));
  }
  return el("div", { id: "transaction-details", class: "narrow wide-narrow" }, button("Back to activity", () => navigate("activity"), "text", "back"), heading("Transfer details", `${netLabel()} mainnet`),
    card(statusLabel(p), `${p.prepared.request.amount} ${p.prepared.request.asset} on ${netLabel()}`, pill(statusLabel(p), statusTone(p)),
      el("div", { class: "details-list" }, detail("Recipient", p.prepared.request.to, true), detail("Amount", `${p.prepared.request.amount} ${p.prepared.request.asset}`), detail("Network", `${netLabel()} · ${chain}`), detail("Nonce", String(p.prepared.request.nonce)), p.signed ? detail("Transaction hash", p.signed.hash, true) : null,
        p.receipt ? detail("Block", String(p.receipt.block_number)) : null,
        p.receipt ? detail("Actual total network fee", `${formatUnits(p.receipt.actual_fee_wei, 18)} ETH`) : null,
        p.receipt && chain === 8453 ? detail("Actual L1 data / operator fees", `${formatUnits(p.receipt.actual_l1_fee_wei, 18)} / ${formatUnits(p.receipt.actual_operator_fee_wei, 18)} ETH`) : null),
      p.resolution_note ? info(p.resolution_note, "warning") : null,
      p.receipt?.additional_fee_exceeded_estimate ? info("Base’s additional fees exceeded the earlier estimate. The actual fee above is retained; the payment outcome is recorded independently.", "warning") : null,
      p.phase === "signed" || p.phase === "unknown" ? info("Signed bytes exist, but submission is not confirmed. The transaction may already have reached the network. A retry uses those identical bytes and the same nonce.") : null,
      p.execution === "failed" ? info("The transaction reverted. The transfer did not execute, but the network charged gas.", "warning") : null,
      ...actions));
}
function retryView(): HTMLElement {
  const payment = currentPayment;
  if (!payment?.signed) return activityView();
  const credential = accessField();
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (!form.reportValidity()) return;
    let value = takeInput(credential.input);
    let access: Awaited<ReturnType<typeof operationCredential>> | null = null;
    void run("Reconciling and retrying saved bytes…", async authorized => {
      try {
        access = await operationCredential(vault!, value, authorized);
        if (!authorized() || !unlocked) return;
        const permit = access;
        permit.secret = ""; // Retry authorizes saved bytes; it never needs a signing secret.
        const result = await engine().retry(payment.intent_id, async () => unlocked && await permit.current() && unlocked);
        if (authorized()) { currentPayment = result.record; route = "transaction"; state().records = await journal.list(chain, vault!.address); }
      } finally { if (access) access.secret = ""; value = ""; }
    });
  } }, credential.node, el("button", { type: "submit", class: "button primary wide" }, "Reconcile, then retry identical bytes"));
  return el("div", { class: "narrow" }, button("Back to transfer", () => navigate("transaction"), "text", "back"), heading("Retry a submission", "No new payment or signature is created."), card("Use the exact saved transaction", "The wallet checks for a receipt first. If it is still unconfirmed, only the same bytes and nonce are sent again.", detail("Transaction", payment.signed.hash, true), detail("Amount", `${payment.signed.amount} ${payment.signed.asset}`), form));
}
function profileView(): HTMLElement {
  return el("div", { class: "profile-page" }, button("Back", () => navigate("assets"), "text", "back"), heading("Profile", "Your wallet on this device."),
    el("div", { class: "profile-intro settings-account" }, el("span", { class: "account-avatar", "aria-hidden": "true" }, el("span"), el("span"), el("span"), el("span")), el("div", {}, el("strong", {}, displayNames.walletName), el("span", {}, displayNames.accountName), el("span", { class: "mono break" }, vault!.address))),
    button("Copy address", () => void copyAddress(), "secondary wide", "copy"),
    el("section", { class: "settings-list" },
      el("button", { type: "button", class: "settings-row", onclick: () => navigate("rename") }, icon("wallet"), el("span", {}, el("strong", {}, "Wallet & account names"), el("small", {}, "Make this wallet yours")), icon("chevron")),
      el("button", { type: "button", class: "settings-row", onclick: () => navigate("settings") }, icon("settings"), el("span", {}, el("strong", {}, "Settings"), el("small", {}, "App lock and recovery phrase")), icon("chevron"))));
}
function renameView(): HTMLElement {
  const walletName = input("text", displayNames.walletName, { id: "wallet-name", autocomplete: "off", maxlength: 64, required: true });
  const accountName = input("text", displayNames.accountName, { id: "account-name", autocomplete: "off", maxlength: 64, required: true });
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (busy || !vault || !form.reportValidity()) return;
    try {
      displayNames = saveDisplayNames(vault.address, { walletName: walletName.value, accountName: accountName.value });
      error = ""; notice = "Names saved."; route = "profile"; render();
    } catch (failure) { error = failure instanceof Error ? failure.message : "The names could not be saved."; renderMessages(); }
  } }, field("Wallet name", walletName), field("Account name", accountName),
    el("p", { class: "field-help" }, "Up to 32 characters each. These names are saved on this device."),
    el("button", { type: "submit", class: "button primary wide" }, "Save names"));
  return el("div", { class: "narrow" }, button("Back", () => navigate("profile"), "text", "back"), heading("Make it yours", "Name your wallet and account."), el("section", { class: "card" }, form));
}
function settingsView(): HTMLElement {
  return el("div", { class: "settings-page" }, button("Back to profile", () => navigate("profile"), "text", "back"), heading("Settings", "Choose how you protect this wallet."),
    el("h2", { class: "settings-label" }, "Security"),
    el("section", { class: "settings-list" },
      el("button", { type: "button", class: "settings-row", onclick: () => navigate("app-lock") }, icon("lock"), el("span", {}, el("strong", {}, "App lock"), el("small", {}, lockLabel())), icon("chevron")),
      el("button", { type: "button", class: "settings-row", onclick: () => navigate("reveal") }, icon("key"), el("span", {}, el("strong", {}, "Recovery phrase"), el("small", {}, "View, copy or take a screenshot")), icon("chevron")),
      lockStatus.mode === "off" ? null : el("div", { class: "settings-row" }, icon("shield"), el("span", {}, el("strong", {}, "Auto-lock"), el("small", {}, "After 2 minutes idle or when this tab is hidden"))),
      lockStatus.mode === "off" ? null : el("button", { type: "button", class: "settings-row", onclick: lock }, icon("lock"), el("span", {}, el("strong", {}, "Lock now"), el("small", {}, lockStatus.mode === "pin" ? "Require your 6-digit app PIN to reopen" : "Require your existing wallet password to reopen")), icon("chevron"))),
    el("h2", { class: "settings-label" }, "Wallet"),
    el("section", { class: "settings-list" },
      el("div", { class: "settings-row" }, icon("wallet"), el("span", {}, el("strong", {}, "Deposit & send"), el("small", {}, "USDC, EURC and ETH · Base and Ethereum mainnet"))),
      el("div", { class: "settings-row" }, icon("info"), el("span", {}, el("strong", {}, "Local Wallet 0.6.0"), el("small", {}, "Development release · Not independently audited")))),
    el("details", { class: "settings-technical" }, el("summary", {}, "Security & recovery details"),
      el("div", { class: "details-list" }, detail("Vault encryption", "AES-256-GCM"), detail("App lock", lockLabel()), detail("Account path", "m/44′/60′/0′/0/0"), detail("BIP39 passphrase", "Empty; separate from app lock")),
      el("ul", { class: "plain-list" }, el("li", {}, "Recovery words and private keys stay on this device. Your words restore the same account in a compatible wallet."),
        el("li", {}, "When app lock is off, anyone able to use this browser profile can open the wallet and approve transfers. Browser encryption does not provide Android Keystore hardware isolation."),
        el("li", {}, "Only native ETH and Circle-issued USDC / EURC are supported. Each asset row identifies the network holding it."), el("li", {}, "One unresolved transfer per network. Avoid concurrent sends from another app using this account."),
        el("li", {}, "Network services see public addresses and chain requests. This wallet trusts their chain observations; it is not a light client."), el("li", {}, "A compromised browser, extension, device or app delivery can compromise a software wallet."))));
}
function appLockView(): HTMLElement {
  return el("div", { class: "narrow" }, button("Back to settings", () => navigate("settings"), "text", "back"), heading("App lock", "Optional protection for opening your wallet and approving transfers."),
    el("section", { class: "security-choices" },
      el("button", { type: "button", class: "security-choice settings-row", "aria-pressed": lockStatus.mode === "off", onclick: () => { if (lockStatus.mode !== "off") navigate("disable-lock"); } }, icon("wallet"), el("span", {}, el("strong", {}, "Off"), el("small", {}, "Open this wallet without an app PIN")), lockStatus.mode === "off" ? icon("check") : icon("chevron")),
      el("button", { type: "button", class: "security-choice settings-row", "aria-pressed": lockStatus.mode === "pin", onclick: () => navigate("set-pin") }, icon("lock"), el("span", {}, el("strong", {}, "6-digit app PIN"), el("small", {}, lockStatus.mode === "pin" ? "Enabled · change your 6-digit PIN" : "Use exactly 6 digits")), lockStatus.mode === "pin" ? icon("check") : icon("chevron")),
      el("div", { class: "security-choice settings-row", "aria-disabled": "true" }, icon("fingerprint"), el("span", {}, el("strong", {}, "Biometrics"), el("small", {}, "Available in the Android app. Not supported by this web version.")))),
    lockStatus.mode === "password" ? info("Your existing wallet password still protects this wallet. Enter it once when choosing a new lock setting.") : null,
    el("p", { class: "fine-print" }, "Your recovery words restore the wallet if you forget your app PIN. No server can reset it."));
}
function lockSettingsForm(disabling: boolean): HTMLElement {
  const credential = accessField();
  const next = pinField("New 6-digit app PIN");
  const repeat = pinField("Confirm 6-digit app PIN");
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (!form.reportValidity() || !vault) return;
    if (!disabling && next.input.value !== repeat.input.value) { error = "The two 6-digit PINs do not match."; renderMessages(); return; }
    let current = takeInput(credential.input);
    let nextPin: string | null = disabling ? null : takeInput(next.input); repeat.input.value = "";
    const expected = vault;
    void run("Updating app lock…", async authorized => {
      try {
        const saved = await protection.setLock(expected, current, nextPin, authorized);
        protectionChanges?.postMessage({ type: "protection-changed" });
        if (!authorized()) return;
        vault = saved; lockStatus = await protection.status(saved);
        if (!authorized()) return;
        reviewed = null; reviewedPreview = null;
        route = "settings"; notice = disabling ? "App lock is off. Your wallet remains encrypted on this device." : "Your 6-digit app PIN is enabled.";
      } finally { current = ""; nextPin = null; }
    });
  } }, credential.node,
    disabling ? info("Anyone who can use this browser profile will be able to open the wallet and approve a transfer.", "warning") : next.node,
    disabling ? null : repeat.node,
    el("button", { type: "submit", class: "button primary wide" }, disabling ? "Turn app lock off" : "Save 6-digit PIN"));
  return el("div", { class: "narrow" }, button("Back", () => navigate("app-lock"), "text", "back"), heading(disabling ? "Turn app lock off" : "Set your 6-digit app PIN", disabling ? "This changes access on this browser only." : "Use exactly 6 digits. This PIN is separate from your device passcode."), card(disabling ? "Confirm your choice" : "App PIN", "Your recovery words and account address stay the same.", form));
}
function revealView(): HTMLElement {
  const credential = accessField();
  const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
    event.preventDefault(); if (!form.reportValidity()) return;
    let value = takeInput(credential.input);
    let access: Awaited<ReturnType<typeof operationCredential>> | null = null;
    void run("Opening your recovery words…", async authorized => {
      try {
        access = await operationCredential(vault!, value, authorized);
        if (!authorized()) return;
        const words = await core.reveal(access.vault, access.secret);
        if (await access.current()) { phrase = words; route = "words"; }
      } finally { if (access) access.secret = ""; value = ""; }
    });
  } }, credential.node, el("button", { type: "submit", class: "button primary wide" }, "Show recovery words", icon("key")));
  return el("div", { class: "narrow" }, button("Back to settings", () => navigate("settings"), "text", "back"), heading("Recovery words", "You can copy the words or take a screenshot."), card("Keep your backup private", "Anyone with these words can move your assets, even if your app PIN is enabled.", form));
}
function render(): void {
  if (!networks.length) return;
  const routeChanged = renderedRoute !== route;
  let content: HTMLElement;
  switch (route) {
    case "welcome": content = welcome(); break;
    case "create": content = createRestore(false); break;
    case "restore": content = createRestore(true); break;
    case "unlock": content = unlockView(); break;
    case "backup": content = backupView(); break;
    case "challenge": content = challengeView(); break;
    case "choose-deposit": content = chooseAssetView("deposit"); break;
    case "choose-send": content = chooseAssetView("send"); break;
    case "receive": content = receiveView(); break;
    case "send": content = sendView(); break;
    case "review": content = reviewView(); break;
    case "asset": content = assetView(); break;
    case "activity": content = activityView(); break;
    case "transaction": content = transactionView(); break;
    case "chain-transaction": content = chainTransactionView(); break;
    case "retry": content = retryView(); break;
    case "profile": content = profileView(); break;
    case "rename": content = renameView(); break;
    case "settings": content = settingsView(); break;
    case "app-lock": content = appLockView(); break;
    case "set-pin": content = lockSettingsForm(false); break;
    case "disable-lock": content = lockSettingsForm(true); break;
    case "reveal": content = revealView(); break;
    case "words": content = backupView(true); break;
    default: content = assetsView();
  }
  const messages = el("div", { id: "messages", class: "messages", "aria-live": "polite", role: "status" }, error ? info(error, "warning") : null, notice ? info(notice, "success") : null);
  mount.replaceChildren(topbar(), el("main", { id: "main", class: `app-main route-${route}` }, messages, content,
    el("footer", { class: "main-footer" }, el("span", {}, icon("lock"), "Keys stored on this device"), el("span", {}, "Local Wallet · v0.6.0"))), el("div", { id: "busy-status", class: "busy-status", "aria-live": "polite", role: "status" }));
  if (renderedRoute === null) { mount.classList.add("app-open"); setTimeout(() => mount.classList.remove("app-open"), 800); }
  updateBusy();
  updateReviewButton();
  if (route === "send") { patchFee(); patchDraftFeedback(); }
  renderedRoute = route;
  // New screens must start at their amount/recipient summary, including
  // asynchronous transitions to review. Live patches never change scroll.
  if (routeChanged) window.scrollTo({ top: 0, left: 0, behavior: "auto" });
}
async function start(): Promise<void> {
  const opening = openingWordmark();
  document.body.append(opening);
  opening.addEventListener("animationend", () => opening.remove(), { once: true });
  setTimeout(() => opening.remove(), 1000);
  try {
    if (!isSecureContext || !crypto?.getRandomValues || !crypto?.subtle || !globalThis.indexedDB || !globalThis.Worker) throw new Error("Open this wallet over HTTPS in a browser with local storage, WebCrypto and Web Workers enabled.");
    if (window.top !== window.self) throw new Error("Open Local Wallet in its own browser tab before creating or unlocking a wallet.");
    const registry = await core.registry();
    if (registry.networks.length !== 2 || !registry.networks.some(n => n.chain_id === 1) || !registry.networks.some(n => n.chain_id === 8453)) throw new Error("The supported mainnet registry is invalid.");
    networks = registry.networks;
    for (const entry of networks) clients.set(entry.chain_id, new NetworkClient(entry));
    journal = await Journal.open(core);
    protection = new WalletProtection(core, journal);
    vault = await journal.vault();
    if (vault) lockStatus = await protection.status(vault);
    route = vault ? "unlock" : "welcome";
    render();
    if (vault?.backup_confirmed && lockStatus.mode === "off") void resumeFromStorage();
  } catch (failure) {
    mount.replaceChildren(el("main", { class: "startup-error" }, el("div", { class: "brand-symbol" }, icon("wallet")), el("h1", {}, "Local Wallet could not open"), info(failure instanceof Error ? failure.message : "The local wallet could not initialize.", "warning"), el("p", {}, "No wallet operation was performed. Reload after resolving the browser or deployment issue.")));
  }
}
async function resumeFromStorage(): Promise<void> {
  if (!journal || !protection || busy || unlocked || document.visibilityState === "hidden") return;
  await run("Opening saved wallet…", async authorized => {
    const saved = await journal.vault();
    if (!authorized()) return;
    vault = saved;
    if (!saved) { route = "welcome"; return; }
    lockStatus = await protection.status(saved);
    if (!authorized()) return;
    route = "unlock";
    // An unfinished backup needs an explicit Continue tap before displaying words.
    if (saved.backup_confirmed && lockStatus.mode === "off") await openSavedWallet("", authorized);
  });
}
for (const type of ["pointerdown", "keydown", "input"]) document.addEventListener(type, () => { lastInteraction = Date.now(); }, { passive: true });
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") { if (unlocked || busy || phrase) lock(); }
  else void resumeFromStorage();
});
window.addEventListener("pagehide", lock);
window.addEventListener("pageshow", () => { if (journal && protection) void resumeFromStorage(); });
if (protectionChanges) protectionChanges.onmessage = event => {
  if (event.data?.type !== "protection-changed" || !journal) return;
  lock();
  void resumeFromStorage();
};
window.addEventListener("online", () => { if (canRead()) refreshWallet(true); });
window.addEventListener("storage", () => {
  if (!unlocked || !vault) return;
  displayNames = readDisplayNames(vault.address); patchLive();
});
setInterval(() => {
  if (unlocked && Date.now() - lastInteraction >= 120_000 && (lockStatus.mode !== "off" || Boolean(phrase))) { lock(); return; }
  if (canRead() && !busy) refreshWallet();
}, 20_000);
void start();
