import QRCode from "qrcode";
import { walletHeader, walletOverview, patchWalletOverview, assetOverview, patchAssetOverview, assetPicker, patchAssetPicker, portfolioConnectionStrip, connectionStrip, activityList, feePanel, openingWordmark, assetBadge, chainBadge, el, button, card, detail, field, input, icon, info, type WalletSection, type DisplayAsset, type DisplayChain, type ConnectionProps, type PortfolioNetworkView, type AssetRef, type OverviewProps } from "../src/ui";

// Isolated visual fixture. This entry imports no main.ts, cryptography, vault,
// journal or transport and is never included in the production build.
const address = "0x000000000000000000000000000000000000dEaD";
type PreviewRoute = WalletSection | "asset" | "send" | "review" | "receive" | "welcome" | "unlock" | "pin" | "words" | "choose-deposit" | "choose-send" | "rename";
let section: PreviewRoute = "assets";
let renderedSection: PreviewRoute | null = null;
let selectedAsset: DisplayAsset = "USDC";
let chain: DisplayChain = 8453;
let state: ConnectionProps["state"] = "live";
let ethereumState: ConnectionProps["state"] | null = null;
let hidden = false;
let feeState = "ready";
let lockEnabled = false;
let walletName = "Wallet 1", accountName = "Account 1";
let largeBalances = false;
let revision = 0;
let refreshingFixture = false;
const mount = document.getElementById("preview")!;
const navigate = (next: PreviewRoute) => { section = next; if (next === "unlock") lockEnabled = true; render(); };
const netLabel = () => chain === 8453 ? "Base" : "Ethereum";
const networkState = (id: DisplayChain) => id === 1 ? ethereumState ?? state : state;
function balances(id: DisplayChain = chain): Record<DisplayAsset, string | null> {
  if (networkState(id) === "unavailable" || networkState(id) === "loading") return { USDC: null, EURC: null, ETH: null };
  if (largeBalances) return { USDC: id === 8453 ? "123456789012345678901234.123456" : "2.000001", EURC: "98765432109876543210.000001", ETH: "123456789012345.000000000000000001" };
  return { USDC: id === 8453 ? revision % 2 ? "0.02" : "0.01" : "0.02", EURC: "0", ETH: "0" };
}
function fixtureNetworks(): PortfolioNetworkView[] {
  return ([8453, 1] as const).map(chainId => ({ chainId, balances: balances(chainId), state: networkState(chainId), refreshing: refreshingFixture || networkState(chainId) === "loading", updatedAt: ["loading", "unavailable"].includes(networkState(chainId)) ? null : 1793871000 + revision, blockNumber: networkState(chainId) === "unavailable" ? null : chainId === 8453 ? 53535211 + revision : 23001234 + revision, provider: "Synthetic interface fixture", error: networkState(chainId) === "unavailable" ? "This network is unavailable in the selected preview state. No request was made." : null }));
}
function openAsset(ref: AssetRef, target: "asset" | "send" | "receive"): void { selectedAsset = ref.symbol; chain = ref.chainId; navigate(target); }
function heading(title: string, subtitle: string): HTMLElement {
  return el("div", { class: "page-heading" }, el("div", {}, el("h1", {}, title), el("p", { class: "muted" }, subtitle)));
}
function connection(): HTMLElement {
  const view = fixtureNetworks().find(network => network.chainId === chain)!;
  return connectionStrip({ ...view, networkName: netLabel(), onRefresh: () => { state = "live"; ethereumState = null; patchPresentation(); } });
}
function portfolioConnection(): HTMLElement {
  return portfolioConnectionStrip({ networks: fixtureNetworks(), onRefresh: () => { state = "live"; ethereumState = null; patchPresentation(); } });
}
function overviewProps(activeTab: "assets" | "activity" = "assets"): OverviewProps {
  return { address, walletName, accountName, networks: fixtureNetworks(), hidden, activeTab,
    onSend: ref => ref ? openAsset(ref, "send") : navigate("choose-send"), onReceive: ref => ref ? openAsset(ref, "receive") : navigate("choose-deposit"), onCopy: () => {},
    onAsset: ref => openAsset(ref, "asset"), onEditNames: () => navigate("rename"), onTogglePrivacy: () => { hidden = !hidden; patchPresentation(); }, onTab: navigate };
}
function fixtureOverview(activeTab: "assets" | "activity" = "assets"): HTMLElement { return walletOverview(overviewProps(activeTab)); }
function pickerProps() {
  return { mode: section === "choose-deposit" ? "deposit" as const : "send" as const, networks: fixtureNetworks(), hidden,
    onSelect: (ref: AssetRef) => openAsset(ref, section === "choose-deposit" ? "receive" : "send"), onBack: () => navigate("assets") };
}
function assetProps() {
  return { chainId: chain, symbol: selectedAsset, amount: balances()[selectedAsset], loading: networkState(chain) === "loading", stale: networkState(chain) === "stale", hidden, onSend: () => navigate("send"), onReceive: () => navigate("receive") };
}
function patchPresentation(): void {
  const overview = document.getElementById("wallet-overview");
  if (overview) patchWalletOverview(overview, overviewProps(section === "activity" ? "activity" : "assets"));
  const chooser = document.getElementById("asset-picker");
  if (chooser) patchAssetPicker(chooser, pickerProps());
  const asset = document.getElementById("asset-overview");
  if (asset) patchAssetOverview(asset, assetProps());
  const strip = document.getElementById("connection-strip");
  if (strip) strip.replaceWith(section === "assets" || section === "activity" || section.startsWith("choose-") ? portfolioConnection() : connection());
}
function refreshCycle(): void {
  if (refreshingFixture) return;
  refreshingFixture = true; patchPresentation();
  setTimeout(() => { revision++; state = "live"; ethereumState = null; refreshingFixture = false; patchPresentation(); }, 2000);
}
function playOpening(): void {
  document.querySelector(".opening-wordmark")?.remove();
  const opening = openingWordmark(); document.body.append(opening);
  opening.addEventListener("animationend", () => opening.remove(), { once: true });
  setTimeout(() => opening.remove(), 1000);
}
function fixtureActivity(): HTMLElement {
  if (fixtureNetworks().every(network => network.state === "loading")) return el("div", { class: "activity-loading" }, ...[1, 2].map(() => el("div", { class: "activity-skeleton" }, el("span"), el("span"), el("span"))));
  if (fixtureNetworks().every(network => network.state === "unavailable")) return el("div", { class: "empty-state" }, icon("history"), el("h3", {}, "Recent activity is unavailable"), el("p", {}, "Find a transfer by its transaction hash or try refreshing."));
  return activityList([
    { id: "fixture-1", chainId: 8453, direction: "incoming" as const, symbol: "USDC" as const, amount: "0.01", counterparty: "0x1111111111111111111111111111111111111111", status: "Confirmed", statusTone: "success" as const, timestamp: 1793870900, onOpen: () => navigate("activity") },
    { id: "fixture-2", chainId: 1, direction: "incoming" as const, symbol: "USDC" as const, amount: "0.02", counterparty: "0x2222222222222222222222222222222222222222", status: "Confirmed", statusTone: "success" as const, timestamp: 1793870800, onOpen: () => navigate("activity") },
  ], hidden);
}
function fixtureFee(): HTMLElement {
  return feePanel({
    state: feeState === "error" ? "error" : feeState === "loading" ? "loading" : feeState === "empty" ? "empty" : "ready",
    estimatedFeeEth: feeState === "ready" || feeState === "shortage" ? "0.000000327" : null,
    budgetEth: "0.000000718", executionFeeEth: "0.000000182", l1FeeEth: "0.000000145", operatorFeeEth: "0", isBase: chain === 8453,
    error: feeState === "error" ? "The network fee could not be estimated. Try again shortly." : null, warnings: [],
    shortages: feeState === "shortage" ? ["ETH needed for fees: 0.000000718 ETH on Base."] : [],
    source: "Synthetic interface fixture. Not a live quote.", onEstimate: () => { feeState = "ready"; render(); },
  });
}
function render(): void {
  const sectionChanged = renderedSection !== section;
  const unlocked = section !== "welcome" && section !== "unlock";
  const header = walletHeader({ unlocked, active: section === "activity" ? "activity" : section === "settings" ? "settings" : ["profile", "pin", "words", "rename"].includes(section) ? "profile" : "assets", walletName, accountName, lockEnabled, onNavigate: navigate, onHome: () => navigate(unlocked ? "assets" : "welcome"), onLock: () => navigate("unlock") });
  let content: HTMLElement;
  if (section === "choose-deposit" || section === "choose-send") {
    content = el("div", {}, assetPicker(pickerProps()), portfolioConnection());
  } else if (section === "send") {
    const amount = input("text", "0.01", { id: "send-amount", class: "amount-input", inputmode: "decimal", "aria-label": "Amount" });
    const asset = button(selectedAsset, () => navigate("choose-send"), "text compact");
    asset.append(icon("down"));
    asset.id = "send-asset";
    asset.setAttribute("aria-label", "Change asset and network");
    content = el("div", { class: "send-layout" }, el("div", { class: "narrow send-column" }, button("Back", () => navigate("assets"), "text", "back"), heading("Send", "Transfer assets on " + netLabel() + "."),
      el("section", { class: "card send-card" }, el("div", { class: "form-stack" },
        el("div", { class: "transfer-network" }, chainBadge(chain), el("strong", {}, netLabel() + " mainnet"), el("span", {}, "Fees paid in ETH")),
        field("Recipient address", input("text", "0x1111111111111111111111111111111111111111", { class: "mono", id: "send-recipient" }), "The recipient must support " + netLabel() + ". Check the full address."),
        el("div", { class: "amount-entry" }, el("div", { class: "amount-entry-head" }, el("label", { for: "send-amount" }, "Amount"), el("div", { class: "asset-picker" }, assetBadge(selectedAsset), asset)), amount, el("span", { class: "available-balance" }, "0.01 " + selectedAsset + " available on " + netLabel())),
        fixtureFee(), feeState === "shortage" ? button("Deposit ETH on " + netLabel(), () => { selectedAsset = "ETH"; navigate("receive"); }, "text compact", "receive") : null,
        el("button", { type: "button", class: "button primary wide", disabled: feeState !== "ready", onclick: () => navigate("review") }, "Review transfer", icon("arrow")),
        el("p", { class: "form-caption" }, "Review the recipient and fee before you confirm."))), connection()));
  } else if (section === "review") {
    content = el("div", { class: "narrow review-page" }, button("Edit transfer", () => navigate("send"), "text", "back"), heading("Confirm transfer", "Check the recipient and network before sending."),
      el("section", { class: "card" }, el("div", { class: "review-summary" },
        el("div", { class: "review-amount" }, assetBadge(selectedAsset), el("strong", {}, "0.01"), el("span", {}, selectedAsset)),
        el("div", { class: "review-network" }, chainBadge(chain), netLabel() + " mainnet"),
        detail("To", "0x1111111111111111111111111111111111111111", true), detail("Estimated network fee", "0.000000327 ETH"), detail("Network fee budget", "0.000000718 ETH"),
        el("p", { class: "fee-help" }, "This budget includes a buffer. The actual fee may be lower."),
        el("details", { class: "fee-details" }, el("summary", {}, "Transaction & fee details"), detail("From", address, true), detail("Nonce", "0"))),
        el("div", { class: "form-stack" }, el("label", { class: "check-label" }, input("checkbox"), el("span", {}, "I checked the full recipient address and the " + netLabel() + " network.")),
          lockEnabled ? field("6-digit app PIN", input("password", "", { disabled: true, placeholder: "······", class: "pin-input", inputmode: "numeric", maxlength: 6 }), "Use the six numbers you set in Profile → Settings.") : el("p", { class: "fine-print" }, "App lock is off. Check the transfer details before confirming."),
          el("button", { type: "button", class: "button primary wide", disabled: true }, icon(lockEnabled ? "lock" : "send"), "Confirm and send " + selectedAsset))));
  } else if (section === "receive") {
    const asset = button(selectedAsset, () => navigate("choose-deposit"), "text");
    asset.append(icon("down"));
    asset.setAttribute("aria-label", "Change deposit asset and network");
    const canvas = el("canvas", { class: "qr", role: "img", "aria-label": "Preview-only QR; not a payment request" });
    void QRCode.toCanvas(canvas, "Local Wallet interface preview. Not a payment request. Do not send funds.", { width: 224, margin: 3, errorCorrectionLevel: "M", color: { dark: "#0B0D10", light: "#FFFFFF" } });
    content = el("div", { class: "narrow receive-page" }, button("Back", () => navigate("choose-deposit"), "text", "back"), heading("Deposit", selectedAsset + " on " + netLabel() + "."),
      el("section", { class: "card receive-card" }, el("div", { class: "receive-select" }, assetBadge(selectedAsset),
        asset, el("span", { class: "network-label-pill" }, chainBadge(chain), netLabel())),
        el("div", { class: "qr-wrap" }, canvas), el("p", { class: "field-label" }, "Your wallet address"), el("p", { class: "full-address mono" }, address),
        button("Copy address", () => {}, "primary wide", "copy"),
        el("p", { class: "receive-note" }, "Only send ", el("strong", {}, selectedAsset + " on " + netLabel()), " to this address. Select the same network in the sending wallet."),
        el("details", { class: "fee-details" }, el("summary", {}, selectedAsset + " token details"), detail("Contract", "Synthetic preview — no payment address"))),
      connection(), el("p", { class: "watching-note" }, icon("activity"), "Checking for incoming transfers automatically."));
  } else if (section === "rename") {
    const wallet = input("text", walletName, { required: true, maxlength: 64, autocomplete: "off" });
    const account = input("text", accountName, { required: true, maxlength: 64, autocomplete: "off" });
    const validation = el("p", { class: "field-error", role: "status" });
    const form = el("form", { class: "form-stack", onsubmit: (event: Event) => {
      event.preventDefault();
      const nextWallet = wallet.value.trim(), nextAccount = account.value.trim();
      if (!nextWallet || !nextAccount || Array.from(nextWallet).length > 32 || Array.from(nextAccount).length > 32) { validation.textContent = "Use 1–32 characters for each name."; return; }
      walletName = nextWallet; accountName = nextAccount; navigate("profile");
    } }, field("Wallet name", wallet), field("Account name", account), validation,
      el("p", { class: "field-help" }, "Up to 32 characters each. Preview names stay in memory only."),
      el("button", { type: "submit", class: "button primary wide" }, "Save names"));
    content = el("div", { class: "narrow" }, button("Back", () => navigate("profile"), "text", "back"), heading("Make it yours", "Name your wallet and account."), el("section", { class: "card" }, form));
  } else if (section === "profile") {
    content = el("div", { class: "profile-page" }, button("Back", () => navigate("assets"), "text", "back"), heading("Profile", "Manage your wallet and preferences."),
      el("div", { class: "profile-intro" }, el("div", { class: "settings-account" }, el("span", { class: "account-avatar" }, el("span"), el("span"), el("span"), el("span")), el("div", {}, el("strong", {}, walletName + " / " + accountName), el("span", { class: "mono" }, address)))),
      el("section", { class: "settings-list" },
        el("button", { type: "button", class: "settings-row", onclick: () => navigate("rename") }, icon("edit"), el("span", {}, el("strong", {}, "Wallet & account names"), el("small", {}, "Make it yours")), icon("chevron")),
        el("button", { type: "button", class: "settings-row", onclick: () => navigate("settings") }, icon("settings"), el("span", {}, el("strong", {}, "Settings"), el("small", {}, "App lock, recovery and preferences")), icon("chevron")),
        el("button", { type: "button", class: "settings-row", onclick: () => navigate("words") }, icon("key"), el("span", {}, el("strong", {}, "Recovery phrase"), el("small", {}, "View or copy your 24 recovery words")), icon("chevron"))),
      el("p", { class: "fine-print" }, "USDC, EURC and ETH · Base and Ethereum mainnet"));
  } else if (section === "settings") {
    content = el("div", { class: "settings-page" }, button("Back to profile", () => navigate("profile"), "text", "back"), heading("Settings", "Security and preferences for this device."),
      el("h2", { class: "settings-label" }, "Security"),
      el("section", { class: "settings-list" },
        el("button", { type: "button", class: "settings-row", onclick: () => navigate("pin") }, icon("lock"), el("span", {}, el("strong", {}, "App lock"), el("small", {}, lockEnabled ? "6-digit PIN · Required to unlock and send" : "Off · No app PIN required")), icon("chevron")),
        el("div", { class: "settings-row" }, icon("fingerprint"), el("span", {}, el("strong", {}, "Biometrics"), el("small", {}, "Available in the Android app"))),
        el("button", { type: "button", class: "settings-row", onclick: () => navigate("words") }, icon("key"), el("span", {}, el("strong", {}, "Recovery phrase"), el("small", {}, "24 words · Copy and screenshots allowed")), icon("chevron")),
        lockEnabled ? el("div", { class: "settings-row" }, icon("shield"), el("span", {}, el("strong", {}, "Auto-lock"), el("small", {}, "After 2 minutes idle or when this tab is hidden"))) : null),
      el("h2", { class: "settings-label" }, "Wallet"),
      el("section", { class: "settings-list" },
        el("div", { class: "settings-row" }, icon("wallet"), el("span", {}, el("strong", {}, "Deposit & send"), el("small", {}, "USDC, EURC and ETH · Base and Ethereum mainnet"))),
        el("div", { class: "settings-row" }, icon("info"), el("span", {}, el("strong", {}, "Local Wallet 0.6.0"), el("small", {}, "Development release · Not independently audited")))),
      el("details", { class: "settings-technical" }, el("summary", {}, "Security & recovery details"), el("p", { class: "fine-print" }, "This visual fixture contains no wallet or secrets. Create, restore, unlock and sign are unavailable here.")));
  } else if (section === "pin") {
    content = el("div", { class: "narrow settings-page" }, button("Back to settings", () => navigate("settings"), "text", "back"), heading("App lock", "An optional lock for this wallet on this device."),
      el("div", { class: "security-choices" },
        el("button", { type: "button", class: "security-choice", "aria-pressed": String(!lockEnabled), onclick: () => { lockEnabled = false; render(); } }, icon("wallet"), el("span", {}, el("strong", {}, "No app lock"), el("small", {}, "Open your wallet without an app PIN. Keep this device private.")), !lockEnabled ? icon("check") : null),
        el("button", { type: "button", class: "security-choice", "aria-pressed": String(lockEnabled), onclick: () => { lockEnabled = true; render(); } }, icon("lock"), el("span", {}, el("strong", {}, "6-digit app PIN"), el("small", {}, "Use exactly six numbers to unlock, reveal words and send.")), lockEnabled ? icon("check") : null),
        el("button", { type: "button", class: "security-choice", disabled: true }, icon("fingerprint"), el("span", {}, el("strong", {}, "Biometrics"), el("small", {}, "Available in the Android app")))),
      lockEnabled ? el("div", { class: "card pin-setup" }, el("div", { class: "form-stack" },
        field("New 6-digit app PIN", input("password", "", { class: "pin-input", inputmode: "numeric", maxlength: 6, placeholder: "······", autocomplete: "off" })),
        field("Confirm 6-digit app PIN", input("password", "", { class: "pin-input", inputmode: "numeric", maxlength: 6, placeholder: "······", autocomplete: "off" })),
        el("p", { class: "pin-caption" }, "Exactly 6 numbers. This is separate from your phone's screen-lock code."),
        el("button", { type: "button", class: "button primary wide", disabled: true }, "Enable 6-digit PIN"),
        el("p", { class: "fine-print" }, "Interface preview only. No PIN is created or stored."))) : null);
  } else if (section === "words") {
    content = el("div", { class: "narrow" }, button("Back to profile", () => navigate("profile"), "text", "back"), heading("Recovery phrase", "Keep all 24 words in this exact order."),
      el("div", { class: "resume-backup" }, icon("key"), el("span", {}, "Continue your recovery backup. Your existing wallet stays the same.")),
      info("Synthetic layout preview. These placeholders are not recovery words and cannot restore a wallet."),
      el("ol", { class: "word-grid", "aria-label": "Synthetic recovery-word layout" }, ...Array.from({ length: 24 }, (_, index) => el("li", {}, el("span", {}, String(index + 1)), el("strong", {}, `sample${String(index + 1).padStart(2, "0")}`)))),
      el("div", { class: "recovery-actions" }, el("button", { type: "button", class: "button secondary", disabled: true }, icon("copy"), "Copy recovery phrase")),
      el("p", { class: "recovery-caption" }, "Screenshots are allowed. Anyone who sees the words can control the wallet. Keep copies private."),
      button("Continue verification", () => {}, "primary wide"));
  } else if (section === "activity") {
    content = el("div", { class: "activity-page" }, fixtureOverview("activity"),
      el("section", { class: "activity-card" }, el("div", { class: "activity-status" }, el("span", {}, state === "loading" ? "Checking recent transactions…" : state === "unavailable" || ethereumState === "unavailable" ? "Some history could not be refreshed" : "Base + Ethereum"), el("details", { class: "coverage-details" }, el("summary", {}, "Coverage"), el("div", {}, el("p", {}, "Recent verified activity and saved transfers. This is not a complete account index.")))), fixtureActivity()), portfolioConnection(),
      el("details", { class: "lookup-card" }, el("summary", {}, icon("history"), "Find a transfer", icon("down")), el("p", { class: "muted" }, "Missing a deposit? Paste its transaction hash to check it on " + netLabel() + "."), el("div", { class: "lookup-form" }, input("text", "", { placeholder: "Transaction hash · 0x…", class: "mono" }), button("Find transaction", () => {}, "secondary"))),
      el("a", { class: "external-link" }, "View address on explorer", icon("external")));
  } else if (section === "asset") {
    content = el("div", { class: "asset-page narrow" }, button("Back", () => navigate("assets"), "text", "back"), heading(selectedAsset === "USDC" ? "USD Coin" : selectedAsset === "EURC" ? "Euro Coin" : "Ether", selectedAsset),
      assetOverview(assetProps()), connection(),
      el("section", { class: "asset-metadata" }, detail("Network", el("span", { class: "network-label-pill" }, chainBadge(chain), netLabel())), el("details", { class: "fee-details" }, el("summary", {}, "Token details"), detail("Contract", "Preview fixture only"))),
      el("section", { class: "activity-card" }, el("div", { class: "section-title" }, el("h2", {}, "Recent activity"), button("View all", () => navigate("activity"), "text compact", "chevron")), fixtureActivity()));
  } else if (section === "welcome") {
    content = el("div", { class: "welcome" },
      el("div", { class: "welcome-copy" }, el("div", { class: "welcome-emblem" }, icon("wallet")), el("h1", {}, "Your crypto.", el("br"), "Your control."), el("p", { class: "lead" }, "Deposit and send on Base and Ethereum. Your keys stay with you."),
        el("div", { class: "welcome-assets" }, ...(["USDC", "EURC", "ETH"] as const).map(symbol => el("span", {}, assetBadge(symbol), symbol)))),
      el("div", { class: "welcome-actions" }, button("Create a new wallet", () => {}, "primary wide"), button("Import an existing wallet", () => {}, "secondary wide")),
      el("p", { class: "fine-print" }, "Your recovery words restore your wallet. Keep them private and offline."),
      el("p", { class: "fine-print" }, "Mainnet development release · Independent security audit pending."));
  } else if (section === "unlock") {
    content = el("div", { class: "unlock-layout narrow" }, el("div", { class: "hero-symbol" }, icon("lock")), heading("Welcome back", "Enter the 6-digit app PIN you set in Profile → Settings."),
      card("Your wallet is locked", "Use exactly six numbers.", el("div", { class: "form-stack" }, field("6-digit app PIN", input("password", "", { class: "pin-input", inputmode: "numeric", maxlength: 6, placeholder: "······", autocomplete: "off" })), button("Unlock wallet", () => navigate("assets"), "primary wide", "lock")),
        el("p", { class: "fine-print" }, "This preview does not check or save your input. App lock is optional for new wallets.")));
  } else {
    content = el("div", { class: "dashboard" }, fixtureOverview(), portfolioConnection());
  }
  const controls = el("div", { class: "preview-states", "aria-label": "Synthetic preview controls" },
    ...(["assets", "activity", "asset", "choose-deposit", "choose-send", "send", "review", "receive", "profile", "rename", "settings", "pin", "words", "welcome", "unlock"] as const).map(value => button(value, () => navigate(value), "quiet")),
    ...(["live", "loading", "stale", "unavailable"] as const).map(value => button(value, () => { state = value; ethereumState = null; render(); }, "quiet")),
    button("Ethereum unavailable", () => { state = "live"; ethereumState = "unavailable"; render(); }, "quiet"),
    button("Refresh cycle (2 s)", refreshCycle, "quiet"),
    button("Toggle large balances", () => { largeBalances = !largeBalances; patchPresentation(); }, "quiet"),
    button("Long names (32)", () => { walletName = "W".repeat(32); accountName = "M".repeat(32); render(); }, "quiet"),
    button("Replay opening", playOpening, "quiet"),
    button("Reset fixture", () => { state = "live"; ethereumState = null; largeBalances = false; hidden = false; revision = 0; walletName = "Wallet 1"; accountName = "Account 1"; navigate("assets"); }, "quiet"),
    ...(section === "send" ? ["empty", "loading", "ready", "shortage", "error"].map(value => button("Fee: " + value, () => { feeState = value; render(); }, "quiet")) : []));
  const footer = el("footer", { class: "main-footer" }, el("span", {}, icon("lock"), "Keys stored on this device"), el("span", {}, "Local Wallet · v0.6.0"));
  mount.replaceChildren(header, el("main", { class: "app-main", id: "main" }, content, footer, controls));
  renderedSection = section;
  if (sectionChanged) window.scrollTo({ top: 0, left: 0, behavior: "auto" });
}
mount.classList.add("app-open");
render();
playOpening();
