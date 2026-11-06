import { totalUsdc } from "./portfolio";
import type { AssetRef } from "./portfolio";
export type { AssetRef } from "./portfolio";

export type Child = Node | string | null | undefined | false;
let nextFieldId = 0;
export function el<K extends keyof HTMLElementTagNameMap>(tag: K, attributes: Record<string, unknown> = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attributes)) {
    if (value === undefined || value === null) continue;
    if (key.startsWith("aria-") && typeof value === "boolean") {
      node.setAttribute(key, String(value));
      continue;
    }
    if (value === false) continue;
    if (key === "class") node.className = String(value);
    else if (key.startsWith("on") && typeof value === "function") node.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    else if (key === "checked" && node instanceof HTMLInputElement) node.checked = Boolean(value);
    else if (key === "value" && (node instanceof HTMLInputElement || node instanceof HTMLSelectElement || node instanceof HTMLTextAreaElement)) node.value = String(value);
    else if (key === "disabled" && "disabled" in node) (node as HTMLButtonElement).disabled = Boolean(value);
    else node.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children) if (child !== null && child !== undefined && child !== false) node.append(child);
  return node;
}
export function icon(name: string, className = ""): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24"); svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.8"); svg.setAttribute("stroke-linecap", "round"); svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true"); svg.setAttribute("class", `icon ${className}`);
  const paths: Record<string, string[]> = {
    wallet: ["M4 5h14a2 2 0 0 1 2 2v13H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h12v2", "M20 10h-6v6h6", "M16.5 13h.01"],
    assets: ["M3 3h7v7H3z", "M14 3h7v7h-7z", "M3 14h7v7H3z", "M14 14h7v7h-7z"],
    activity: ["M3 12h4l3-8 4 16 3-8h4"],
    settings: ["M9.5 3h5l.6 2.4 2.1 1.2 2.4-.7 2.5 4.2-1.8 1.8v2.4l1.8 1.8-2.5 4.2-2.4-.7-2.1 1.2L14.5 23h-5l-.6-2.2-2.1-1.2-2.4.7-2.5-4.2 1.8-1.8v-2.4L1.9 10l2.5-4.2 2.4.7 2.1-1.2L9.5 3Z", "M15.5 13a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0"],
    profile: ["M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0", "M4 21v-2a8 8 0 0 1 16 0v2"],
    fingerprint: ["M7 8a6 6 0 0 1 11 3v3", "M4 11a8 8 0 0 1 2-5", "M4 14c0 3-1 5-2 6", "M8 12a4 4 0 0 1 8 0v3c0 3-1 5-2 7", "M12 11v4c0 3-1 5-3 7", "M8 15c0 2-1 5-3 7", "M20 12v3c0 3-.5 5-1 7"],
    eye: ["M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z", "M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0"],
    hidden: ["m3 3 18 18", "M10.5 5.1 12 5c6.5 0 10 7 10 7a20 20 0 0 1-3 3.7", "M6.1 6.1A23 23 0 0 0 2 12s3.5 7 10 7c2.2 0 4.1-.8 5.7-1.9", "M10 10a3 3 0 0 0 4 4"],
    down: ["m6 9 6 6 6-6"],
    close: ["m6 6 12 12", "M18 6 6 18"],
    history: ["M3 5v5h5", "M4 10a8 8 0 1 1 1 8", "M12 8v5l3 2"],
    gas: ["M4 3h10v18H4z", "M4 9h10", "m16 5 3 3v9a2 2 0 0 1-4 0v-4h-1", "M2 21h14"],
    arrow: ["M5 12h14", "m13 6 6 6-6 6"],
    send: ["M6 18 18 6", "M6 6h12v12"],
    receive: ["M18 6 6 18", "M18 18H6V6"],
    edit: ["m15 4 5 5", "M4 15 16 3a2.1 2.1 0 0 1 3 0l2 2a2.1 2.1 0 0 1 0 3L9 20l-6 1 1-6Z"],
    lock: ["M5 10h14v11H5z", "M8 10V6a4 4 0 0 1 8 0v4", "M12 15v2"],
    shield: ["M12 3 3 7v5c0 5 5 8 9 10 4-2 9-5 9-10V7z", "m8 12 3 3 5-6"],
    copy: ["M9 9h12v12H9z", "M15 9V3H3v12h6"],
    refresh: ["M20 7v5h-5", "M4 17v-5h5", "M6 7a7 7 0 0 1 12-2l2 7", "M4 12l2 7a7 7 0 0 0 12-2"],
    check: ["m5 12 4 4L19 6"],
    chevron: ["m9 5 7 7-7 7"],
    back: ["M19 12H5", "m11 6-6 6 6 6"],
    external: ["M14 3h7v7", "m10 14 11-11", "M10 3H3v18h18v-7"],
    info: ["M12 8h.01", "M12 11v6", "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0"],
    key: ["M8 14a5 5 0 1 1 5-5L22 0", "M16 6l3 3", "M19 3l3 3"],
  };
  for (const d of paths[name] ?? paths.wallet) { const p = document.createElementNS(ns, "path"); p.setAttribute("d", d); svg.append(p); }
  return svg;
}
export function button(text: string, action: () => void, style = "primary", symbol?: string): HTMLButtonElement {
  return el("button", { type: "button", class: `button ${style}`, onclick: action }, symbol ? icon(symbol) : null, text);
}
export function field(label: string, input: HTMLElement, help?: string): HTMLElement {
  const id = input.id || `local-field-${++nextFieldId}`;
  input.id = id;
  return el("div", { class: "field" }, el("label", { for: id }, label), input, help ? el("p", { class: "field-help" }, help) : null);
}
export function pill(text: string, type = "neutral"): HTMLElement { return el("span", { class: `pill ${type}` }, text); }
export function card(title: string, subtitle: string, ...children: Child[]): HTMLElement {
  return el("section", { class: "card" }, el("div", { class: "card-heading" }, el("h2", {}, title), el("p", { class: "muted" }, subtitle)), ...children);
}
export function info(text: string, type = "note"): HTMLElement { return el("div", { class: `notice ${type}` }, icon(type === "success" ? "check" : "info"), el("p", {}, text)); }
export function detail(label: string, value: Child, mono = false): HTMLElement { return el("div", { class: "detail" }, el("span", { class: "muted" }, label), el("div", { class: mono ? "mono break" : "break" }, value)); }
export function input(type: string, value = "", attrs: Record<string, unknown> = {}): HTMLInputElement {
  return el("input", { type, value, ...attrs });
}

// Pure presentation components. They have no wallet, storage, network or
// cryptographic access. The production controller supplies real observations;
// a separate development preview may render these with explicit fixture data.
export type WalletSection = "assets" | "activity" | "profile" | "settings";
export type DisplayAsset = "USDC" | "EURC" | "ETH";
export type DisplayChain = 1 | 8453;
function currencyGlyph(symbol: DisplayAsset): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 40 40");
  svg.setAttribute("aria-hidden", "true");
  if (symbol === "ETH") {
    const faces = [
      ["20 3 9 21 20 16", "#BCC5EB"], ["20 3 20 16 31 21", "#8794C8"],
      ["9 21 20 27 20 16", "#7A86B6"], ["20 16 20 27 31 21", "#586897"],
      ["9 23 20 37 20 29", "#BCC5EB"], ["20 29 20 37 31 23", "#8794C8"],
    ];
    for (const [points, fill] of faces) { const face = document.createElementNS(ns, "polygon"); face.setAttribute("points", points); face.setAttribute("fill", fill); svg.append(face); }
  } else {
    const ring = document.createElementNS(ns, "path");
    ring.setAttribute("d", "M13 7.5a14.3 14.3 0 0 0 0 25M27 7.5a14.3 14.3 0 0 1 0 25");
    ring.setAttribute("fill", "none"); ring.setAttribute("stroke", "white"); ring.setAttribute("stroke-width", "2"); ring.setAttribute("stroke-linecap", "round");
    svg.append(ring);
    const mark = document.createElementNS(ns, "path");
    mark.setAttribute("d", symbol === "USDC" ? "M25 14c-1.1-2.5-8.6-3.3-9.5.6-1.1 4.6 10.3 3.1 9.5 7.7-.7 4.4-8.8 4.7-10.5.9M20 9v22" : "M26 12.7c-7.8-4.4-14.8 2.3-12.3 10 1.7 5.1 7.9 7 12.3 4M10.5 18h12M10.5 22h10.5");
    mark.setAttribute("fill", "none"); mark.setAttribute("stroke", "white"); mark.setAttribute("stroke-width", "2.2"); mark.setAttribute("stroke-linecap", "round"); mark.setAttribute("stroke-linejoin", "round");
    svg.append(mark);
  }
  return svg;
}
export function chainBadge(chain: DisplayChain): HTMLElement {
  const badge = el("span", { class: `chain-mark ${chain === 8453 ? "base" : "ethereum"}`, "aria-hidden": "true" });
  if (chain === 1) badge.append(currencyGlyph("ETH"));
  return badge;
}
export function assetBadge(symbol: DisplayAsset): HTMLElement {
  return el("span", { class: `asset-icon ${symbol.toLowerCase()}`, "aria-hidden": "true" }, currencyGlyph(symbol));
}
export function iconButton(label: string, action: () => void, symbol: string, extra = ""): HTMLButtonElement {
  return el("button", { type: "button", class: `icon-button ${extra}`, "aria-label": label, title: label, onclick: action }, icon(symbol));
}
export interface WalletHeaderProps {
  unlocked: boolean; active: WalletSection; disabled?: boolean; navigationEnabled?: boolean; lockEnabled?: boolean;
  walletName?: string; accountName?: string; onNavigate: (section: WalletSection) => void;
  onLock: () => void; onHome: () => void;
}
export function walletHeader(props: WalletHeaderProps): HTMLElement {
  const lockButton = iconButton("Lock wallet", props.onLock, "lock");
  lockButton.dataset.lock = "true";
  const profile = iconButton("Profile", () => props.onNavigate("profile"), "profile", props.active === "profile" || props.active === "settings" ? "active" : "");
  profile.disabled = Boolean(props.disabled);
  if (props.walletName || props.accountName) profile.title = [props.walletName, props.accountName].filter(Boolean).join(" · ") + " · Profile";
  return el("header", { class: "topbar" },
    el("button", { type: "button", class: "brand", onclick: props.onHome, "aria-label": "Local Wallet home" }, el("span", { class: "brand-symbol" }, icon("wallet")), el("span", {}, "Local Wallet")),
    el("div", { class: "topbar-right" }, props.unlocked && props.navigationEnabled !== false ? profile : null, props.unlocked ? props.lockEnabled !== false ? lockButton : null : el("span", { class: "header-caption" }, icon("shield"), "Self-custody")));
}
export function openingWordmark(): HTMLElement {
  return el("div", { class: "opening-wordmark", "aria-hidden": "true" },
    el("div", { class: "opening-wordmark-inner" }, el("span", { class: "opening-symbol" }, icon("wallet")), el("span", {}, "Local Wallet")));
}
export interface ConnectionProps {
  state: "idle" | "loading" | "live" | "stale" | "unavailable";
  refreshing: boolean; updatedAt: number | null; blockNumber: number | null;
  provider: string | null; error: string | null;
  networkName: string; onRefresh: () => void;
}
export function connectionStrip(props: ConnectionProps): HTMLElement {
  const title = props.state === "live" ? "Up to date" : props.state === "stale" ? "Last saved balance" : props.state === "unavailable" ? "Unable to connect" : "Connecting";
  const time = props.updatedAt ? new Date(props.updatedAt * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : null;
  const refresh = iconButton(props.refreshing ? "Updating balances" : "Refresh balances", props.onRefresh, "refresh", "refresh-control");
  refresh.disabled = props.refreshing;
  if (props.refreshing) refresh.classList.add("is-refreshing");
  return el("section", { id: "connection-strip", class: `connection-strip ${props.state}`, "aria-live": "polite" },
    el("details", { class: "connection-details" }, el("summary", {}, el("span", { class: `status-dot ${props.state === "live" ? "good" : props.state === "stale" || props.state === "unavailable" ? "bad" : ""}` }), el("span", {}, props.refreshing && props.state === "live" ? "Updating…" : title), time ? el("span", { class: "connection-time" }, time) : null, icon("down")),
      el("div", { class: "connection-popover" }, el("strong", {}, `${props.networkName} mainnet`), props.blockNumber !== null ? el("p", {}, `Block ${props.blockNumber.toLocaleString("en-US")}`) : null, props.provider ? el("p", { class: "break" }, `Provider: ${props.provider}`) : null, props.error ? el("p", { class: "connection-error" }, props.error) : null, el("p", {}, "Network balances update automatically. A connection problem does not mean your balance is zero."))), refresh,
    props.state === "stale" || props.state === "unavailable" ? el("p", { class: "connection-explanation" }, props.state === "stale" ? "Reconnecting. The balance shown is your last verified balance." : "Balances are unavailable. Retrying automatically. You can still receive using your address.") : null);
}
export function walletActions(onSend: () => void, onReceive: () => void): HTMLElement {
  return el("div", { class: "wallet-actions", "aria-label": "Transfer actions" },
    el("button", { type: "button", class: "wallet-action action-deposit", onclick: onReceive }, icon("receive"), el("span", {}, "DEPOSIT")),
    el("button", { type: "button", class: "wallet-action action-send", onclick: onSend }, icon("send"), el("span", {}, "SEND")));
}
export function walletTabs(active: "assets" | "activity", onNavigate: (section: "assets" | "activity") => void): HTMLElement {
  return el("nav", { class: "wallet-tabs", "aria-label": "Wallet views" }, ...([["assets", "Assets"], ["activity", "Activity"]] as const).map(([section, label]) => el("button", { type: "button", class: active === section ? "active" : "", "aria-current": active === section ? "page" : undefined, onclick: () => onNavigate(section) }, label)));
}
export interface PortfolioNetworkView {
  chainId: DisplayChain; balances: Record<DisplayAsset, string | null>; state: ConnectionProps["state"];
  refreshing: boolean; updatedAt: number | null; blockNumber: number | null; provider: string | null; error: string | null;
}
const portfolioChains = [8453, 1] as const;
const portfolioAssets: readonly AssetRef[] = (["USDC", "EURC", "ETH"] as const).flatMap(symbol => portfolioChains.map(chainId => ({ chainId, symbol })));
const chainName = (chainId: DisplayChain): string => chainId === 8453 ? "Base" : "Ethereum";
function networkView(networks: readonly PortfolioNetworkView[], chainId: DisplayChain): PortfolioNetworkView | undefined {
  const matches = networks.filter(network => network.chainId === chainId);
  return matches.length === 1 ? matches[0] : undefined;
}
function updateText(node: HTMLElement, value: string): void { if (node.textContent !== value) node.textContent = value; }
function isLoading(network: PortfolioNetworkView | undefined): boolean {
  return Boolean(network && (network.state === "idle" || network.state === "loading"));
}
function isSaved(network: PortfolioNetworkView | undefined): boolean {
  return Boolean(network && (network.state === "stale" || network.state === "unavailable"));
}
interface AssetRowBinding { element: HTMLButtonElement; ref: AssetRef; amount: HTMLElement; caption: HTMLElement; }
function assetRow(ref: AssetRef, onSelect: () => void): AssetRowBinding {
  const amount = el("strong", { class: "tabular" });
  const caption = el("span", { class: "asset-value-caption" });
  const name = `${ref.symbol} (${chainName(ref.chainId)})`;
  const element = el("button", { type: "button", class: "asset-row", "data-asset-key": `${ref.chainId}:${ref.symbol}`, "data-chain-id": ref.chainId, "aria-label": `View ${name}`, onclick: onSelect },
    el("span", { class: "asset-avatar" }, assetBadge(ref.symbol), chainBadge(ref.chainId)),
    el("span", { class: "asset-name" }, el("strong", {}, ref.symbol, " ", el("span", { class: "asset-network-name" }, `(${chainName(ref.chainId)})`)), el("span", {}, ref.symbol === "USDC" ? "USD Coin" : ref.symbol === "EURC" ? "Euro Coin" : "Ether")),
    el("span", { class: "asset-balance" }, amount, caption), icon("chevron"));
  return { element, ref, amount, caption };
}
function patchAssetRow(row: AssetRowBinding, networks: readonly PortfolioNetworkView[], hidden = false): void {
  const network = networkView(networks, row.ref.chainId);
  const amount = network?.balances[row.ref.symbol] ?? null;
  const loading = amount === null && isLoading(network) && !hidden;
  row.element.classList.toggle("stale", amount !== null && isSaved(network));
  row.amount.classList.toggle("row-skeleton", loading);
  row.amount.classList.toggle("unavailable", amount === null && !loading && !hidden);
  updateText(row.amount, hidden ? "••••" : amount ?? (loading ? "" : "—"));
  updateText(row.caption, hidden ? row.ref.symbol : amount === null ? loading ? "Updating" : "Unavailable" : isSaved(network) ? "Last saved" : row.ref.symbol);
  if (loading) row.amount.setAttribute("aria-label", `Loading ${row.ref.symbol} balance on ${chainName(row.ref.chainId)}`);
  else row.amount.removeAttribute("aria-label");
}
export function portfolioConnectionStrip(props: { networks: readonly PortfolioNetworkView[]; onRefresh: () => void }): HTMLElement {
  const states = portfolioChains.map(chainId => ({ chainId, network: networkView(props.networks, chainId) }));
  const unavailable = states.filter(({ network }) => !network || network.state === "unavailable");
  const saved = states.filter(({ network }) => isSaved(network));
  const refreshing = states.some(({ network }) => network?.refreshing || isLoading(network));
  const healthy = states.every(({ network }) => network?.state === "live");
  const title = unavailable.length === states.length ? "Balances unavailable" : unavailable.length === 1 ? `${chainName(unavailable[0]!.chainId)} needs attention` : saved.length ? "Showing saved balances" : healthy && !refreshing ? "Balances up to date" : "Updating balances";
  const refresh = iconButton(refreshing ? "Updating balances" : "Refresh all balances", props.onRefresh, "refresh", "refresh-control");
  refresh.disabled = refreshing;
  refresh.classList.toggle("is-refreshing", refreshing);
  return el("section", { id: "connection-strip", class: "connection-strip portfolio-connection", "aria-live": "polite" },
    el("details", { class: "connection-details" },
      el("summary", {}, el("span", { class: `status-dot ${unavailable.length || saved.length ? "bad" : healthy ? "good" : ""}` }), el("span", {}, title), icon("down")),
      el("div", { class: "connection-popover portfolio-network-status" }, ...states.map(({ chainId, network }) => {
        const updatedAt = network?.updatedAt;
        const status = !network || network.state === "unavailable" ? "Unavailable" : network.state === "stale" ? "Last saved balances" : network.state === "live" ? "Up to date" : "Connecting";
        return el("div", { class: "network-status-item" }, el("div", { class: "network-status-heading" }, chainBadge(chainId), el("strong", {}, chainName(chainId)), el("span", {}, status)),
          updatedAt ? el("p", {}, `Updated ${new Date(updatedAt * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}`) : null,
          network?.blockNumber !== null && network?.blockNumber !== undefined ? el("p", {}, `Block ${network.blockNumber.toLocaleString("en-US")}`) : null,
          network?.provider ? el("p", { class: "break" }, `Provider: ${network.provider}`) : null,
          network?.error ? el("p", { class: "connection-error" }, network.error) : null);
      }), unavailable.length || saved.length ? el("p", {}, "A connection problem does not mean your balance is zero.") : null)), refresh);
}
export interface OverviewProps {
  address: string; walletName: string; accountName: string; networks: readonly PortfolioNetworkView[];
  hidden?: boolean; activeTab?: "assets" | "activity";
  onSend: (asset?: AssetRef) => void; onReceive: (asset?: AssetRef) => void; onAsset: (asset: AssetRef) => void; onCopy: () => void;
  onEditNames?: () => void; onTogglePrivacy?: () => void; onTab?: (section: "assets" | "activity") => void;
}
interface OverviewBinding {
  props: OverviewProps; walletName: HTMLElement; accountName: HTMLElement; address: HTMLButtonElement; addressText: HTMLElement;
  editNames: HTMLButtonElement; privacy: HTMLButtonElement; amount: HTMLElement; coverage: HTMLElement; hero: HTMLElement;
  tabs: HTMLElement; list: HTMLElement; rows: AssetRowBinding[];
}
const overviewBindings = new WeakMap<HTMLElement, OverviewBinding>();
export function walletOverview(props: OverviewProps): HTMLElement {
  let binding: OverviewBinding;
  const walletName = el("span", { class: "wallet-name" });
  const accountName = el("h1", { class: "portfolio-account-name" });
  const addressText = el("span");
  const address = el("button", { type: "button", class: "address-chip", "aria-label": "Copy wallet address", onclick: () => binding.props.onCopy() }, addressText, icon("copy"));
  const editNames = iconButton("Edit wallet and account names", () => binding.props.onEditNames?.(), "edit", "identity-edit");
  const privacy = iconButton("Hide balances", () => binding.props.onTogglePrivacy?.(), "eye", "privacy-control");
  const amount = el("span", { class: "hero-quantity" });
  const coverage = el("span", { class: "hero-coverage" });
  const hero = el("div", { class: "balance-hero" },
    el("div", { class: "balance-eyebrow" }, el("span", {}, "Total USDC"), privacy),
    el("div", { class: "hero-amount tabular" }, amount, el("span", { class: "hero-unit" }, "USDC")),
    el("p", { class: "hero-caption" }, el("span", { class: "chain-pair", "aria-hidden": "true" }, chainBadge(8453), chainBadge(1)), coverage));
  const tabs = walletTabs(props.activeTab ?? "assets", section => binding.props.onTab?.(section));
  const rows = portfolioAssets.map(ref => assetRow(ref, () => binding.props.onAsset(ref)));
  const list = el("div", { class: "asset-list", "aria-label": "Assets on Base and Ethereum" }, ...rows.map(row => row.element));
  const element = el("section", { id: "wallet-overview", class: "wallet-overview" },
    el("div", { class: "account-row" }, el("div", { class: "account-name" }, el("div", { class: "identity-copy" },
      el("div", { class: "identity-eyebrow" }, walletName, editNames), accountName, address))),
    hero, walletActions(() => binding.props.onSend(), () => binding.props.onReceive()), tabs, list);
  binding = { props, walletName, accountName, address, addressText, editNames, privacy, amount, coverage, hero, tabs, list, rows };
  overviewBindings.set(element, binding);
  patchWalletOverview(element, props);
  return element;
}
export function patchWalletOverview(element: HTMLElement, props: OverviewProps): void {
  const binding = overviewBindings.get(element);
  if (!binding) throw new Error("Wallet overview presentation is not initialized.");
  binding.props = props;
  updateText(binding.walletName, props.walletName);
  updateText(binding.accountName, props.accountName);
  updateText(binding.addressText, `${props.address.slice(0, 6)}…${props.address.slice(-4)}`);
  binding.address.title = props.address;
  binding.editNames.hidden = !props.onEditNames;
  binding.privacy.hidden = !props.onTogglePrivacy;
  const privacyLabel = props.hidden ? "Show balances" : "Hide balances";
  if (binding.privacy.getAttribute("aria-label") !== privacyLabel) {
    binding.privacy.setAttribute("aria-label", privacyLabel);
    binding.privacy.title = privacyLabel;
    binding.privacy.replaceChildren(icon(props.hidden ? "hidden" : "eye"));
  }
  const total = totalUsdc(props.networks.map(network => ({ chainId: network.chainId, amount: network.balances.USDC })));
  const missing = portfolioChains.filter(chainId => networkView(props.networks, chainId)?.balances.USDC == null);
  const loading = total === null && missing.length > 0 && missing.every(chainId => isLoading(networkView(props.networks, chainId))) && !props.hidden;
  const saved = portfolioChains.some(chainId => isSaved(networkView(props.networks, chainId)));
  binding.hero.classList.toggle("stale", total !== null && saved);
  binding.amount.classList.toggle("hero-skeleton", loading);
  binding.amount.classList.toggle("is-hidden", Boolean(props.hidden));
  binding.amount.classList.toggle("unavailable", total === null && !loading && !props.hidden);
  updateText(binding.amount, props.hidden ? "••••••" : total ?? (loading ? "" : "—"));
  if (loading) binding.amount.setAttribute("aria-label", "Loading total USDC balance");
  else binding.amount.removeAttribute("aria-label");
  const coverage = total !== null ? saved ? "Base + Ethereum · Includes saved balances" : "Base + Ethereum" : loading ? "Updating Base + Ethereum" : missing.length === 1 ? `${chainName(missing[0]!)} balance unavailable` : "Total unavailable · Check network status";
  updateText(binding.coverage, coverage);
  binding.tabs.hidden = !props.onTab;
  for (const tab of Array.from(binding.tabs.querySelectorAll("button"))) {
    const selected = tab.textContent === (props.activeTab === "activity" ? "Activity" : "Assets");
    tab.classList.toggle("active", selected);
    if (selected) tab.setAttribute("aria-current", "page"); else tab.removeAttribute("aria-current");
  }
  binding.list.hidden = props.activeTab === "activity";
  for (const row of binding.rows) patchAssetRow(row, props.networks, props.hidden);
}
export interface AssetPickerProps {
  mode: "deposit" | "send"; networks: readonly PortfolioNetworkView[]; hidden?: boolean;
  onSelect: (asset: AssetRef) => void; onBack: () => void;
}
interface AssetPickerBinding { props: AssetPickerProps; title: HTMLElement; rows: AssetRowBinding[]; }
const assetPickerBindings = new WeakMap<HTMLElement, AssetPickerBinding>();
export function assetPicker(props: AssetPickerProps): HTMLElement {
  let binding: AssetPickerBinding;
  const title = el("h1");
  const rows = portfolioAssets.map(ref => assetRow(ref, () => binding.props.onSelect(ref)));
  const element = el("section", { id: "asset-picker", class: "asset-picker-page" },
    button("Back", () => binding.props.onBack(), "text", "back"),
    el("div", { class: "page-heading" }, el("div", {}, title, el("p", { class: "muted" }, "Choose an asset and network."))),
    el("div", { class: "asset-list asset-choice-list", "aria-label": "Choose asset and network" }, ...rows.map(row => row.element)));
  binding = { props, title, rows };
  assetPickerBindings.set(element, binding);
  patchAssetPicker(element, props);
  return element;
}
export function patchAssetPicker(element: HTMLElement, props: AssetPickerProps): void {
  const binding = assetPickerBindings.get(element);
  if (!binding) throw new Error("Asset picker presentation is not initialized.");
  binding.props = props;
  updateText(binding.title, props.mode === "deposit" ? "Deposit" : "Send");
  for (const row of binding.rows) {
    row.element.setAttribute("aria-label", `${props.mode === "deposit" ? "Deposit" : "Send"} ${row.ref.symbol} (${chainName(row.ref.chainId)})`);
    patchAssetRow(row, props.networks, props.hidden);
  }
}
export interface AssetOverviewProps {
  chainId: DisplayChain; symbol: DisplayAsset; amount: string | null; loading: boolean; stale: boolean; hidden?: boolean;
  onSend: () => void; onReceive: () => void;
}
interface AssetOverviewBinding {
  props: AssetOverviewProps; badge: HTMLElement; chainMark: HTMLElement; label: HTMLElement;
  amount: HTMLElement; unit: HTMLElement; caption: HTMLElement;
}
const assetOverviewBindings = new WeakMap<HTMLElement, AssetOverviewBinding>();
export function assetOverview(props: AssetOverviewProps): HTMLElement {
  let binding: AssetOverviewBinding;
  const badge = assetBadge(props.symbol), chainMark = chainBadge(props.chainId);
  const label = el("p", { class: "balance-eyebrow" }), amount = el("span", { class: "hero-quantity" });
  const unit = el("span", { class: "hero-unit" }), caption = el("span");
  const element = el("section", { id: "asset-overview", class: "asset-overview" }, badge, label,
    el("div", { class: "hero-amount tabular" }, amount, unit), el("p", { class: "hero-caption" }, chainMark, caption),
    walletActions(() => binding.props.onSend(), () => binding.props.onReceive()));
  binding = { props, badge, chainMark, label, amount, unit, caption };
  assetOverviewBindings.set(element, binding);
  patchAssetOverview(element, props);
  return element;
}
export function patchAssetOverview(element: HTMLElement, props: AssetOverviewProps): void {
  const binding = assetOverviewBindings.get(element);
  if (!binding) throw new Error("Asset overview presentation is not initialized.");
  if (binding.props.symbol !== props.symbol) {
    const badge = assetBadge(props.symbol); binding.badge.replaceWith(badge); binding.badge = badge;
  }
  if (binding.props.chainId !== props.chainId) {
    const badge = chainBadge(props.chainId); binding.chainMark.replaceWith(badge); binding.chainMark = badge;
  }
  binding.props = props;
  element.dataset.assetKey = `${props.chainId}:${props.symbol}`;
  element.classList.toggle("stale", props.stale && props.amount !== null);
  const loading = props.amount === null && props.loading && !props.hidden;
  updateText(binding.label, `Your ${props.symbol} balance`);
  updateText(binding.amount, props.hidden ? "••••••" : props.amount ?? (loading ? "" : "—"));
  updateText(binding.unit, props.symbol);
  updateText(binding.caption, `${chainName(props.chainId)} mainnet${props.stale && props.amount !== null ? " · Last saved balance" : props.amount === null && !props.loading ? " · Unavailable" : ""}`);
  binding.amount.classList.toggle("hero-skeleton", loading);
  binding.amount.classList.toggle("is-hidden", Boolean(props.hidden));
  binding.amount.classList.toggle("unavailable", props.amount === null && !loading && !props.hidden);
  if (loading) binding.amount.setAttribute("aria-label", `Loading ${props.symbol} balance`); else binding.amount.removeAttribute("aria-label");
}
export interface ActivityItemView {
  id: string; chainId: DisplayChain; direction: "incoming" | "outgoing"; symbol: DisplayAsset; amount: string;
  counterparty: string; status: string; statusTone: "success" | "neutral" | "warning";
  timestamp: number | null; title?: string; amountPrefix?: string; onOpen: () => void;
}
export function activityList(items: ActivityItemView[], hideAmounts = false): HTMLElement {
  return el("div", { class: "activity-list" }, ...items.map(item => el("button", { type: "button", class: "activity-row", onclick: item.onOpen, "data-activity-id": item.id, "data-chain-id": item.chainId, "data-activity-key": `${item.chainId}:${item.id}` },
    el("span", { class: `activity-direction ${item.direction}` }, icon(item.direction === "incoming" ? "receive" : "send")),
    el("div", { class: "activity-description" }, el("strong", {}, item.title ?? (item.direction === "incoming" ? `Received ${item.symbol}` : `Sent ${item.symbol}`)), el("span", { class: "activity-network" }, chainBadge(item.chainId), chainName(item.chainId)), el("span", { class: "muted" }, `${item.direction === "incoming" ? "From" : "To"} ${item.counterparty.slice(0, 6)}…${item.counterparty.slice(-4)}${item.timestamp ? ` · ${new Date(item.timestamp * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}` : ""}`)),
    el("div", { class: "activity-value" }, el("strong", { class: `tabular ${item.direction === "incoming" && item.statusTone !== "warning" ? "positive" : ""}` }, hideAmounts ? `•••• ${item.symbol}` : `${item.amountPrefix ?? (item.direction === "incoming" ? "+" : "−")}${item.amount} ${item.symbol}`), pill(item.status, item.statusTone)), icon("chevron"))));
}
export interface FeePanelProps {
  state: "empty" | "loading" | "ready" | "error";
  estimatedFeeEth: string | null; budgetEth: string | null; executionFeeEth: string | null;
  l1FeeEth: string | null; operatorFeeEth: string | null; isBase: boolean;
  error: string | null; warnings: string[]; shortages: string[]; source: string | null;
  onEstimate: () => void;
}
export function feePanel(props: FeePanelProps): HTMLElement {
  return el("section", { id: "fee-panel", class: `fee-panel ${props.state}`, "aria-live": "polite" },
    el("div", { class: "fee-heading" }, el("span", { class: "fee-title" }, icon("gas"), "Cost to transfer"), props.state === "loading" ? el("span", { class: "fee-loading" }, el("span", { class: "spinner" }), "Estimating…") : props.estimatedFeeEth ? el("strong", { class: "tabular" }, `${props.estimatedFeeEth} ETH`) : el("strong", {}, "—")),
    props.state === "empty" ? el("p", { class: "fee-help" }, "Enter a recipient and amount to see the network fee before you continue.") : null,
    props.state === "error" ? el("div", { class: "fee-error" }, el("p", {}, props.error ?? "The fee could not be estimated."), button("Try again", props.onEstimate, "text compact", "refresh")) : null,
    props.state === "ready" ? el("div", {}, el("div", { class: "fee-budget" }, el("span", {}, "ETH needed for fees, including buffer"), el("strong", { class: "tabular" }, `${props.budgetEth} ETH`)),
      ...props.shortages.map(text => el("p", { class: "fee-shortage" }, icon("info"), text)),
      el("details", { class: "fee-details" }, el("summary", {}, "Fee details"), el("div", {},
        detail("Estimated execution", `${props.executionFeeEth} ETH`), props.isBase ? detail("Estimated L1 data fee", `${props.l1FeeEth} ETH`) : null, props.isBase ? detail("Estimated operator fee", `${props.operatorFeeEth} ETH`) : null,
        props.isBase ? el("p", { class: "fine-print" }, "Base’s L1 and operator charges can change. The buffer is a fee budget; only the execution fee has a hard transaction cap.") : el("p", { class: "fine-print" }, "The actual fee may be lower. A reverted transaction still pays for gas."),
        props.source ? el("p", { class: "fine-print" }, props.source) : null,
        ...props.warnings.map(text => el("p", { class: "fine-print" }, text))))) : null);
}
