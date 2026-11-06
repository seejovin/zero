import { CoreClient } from "./core-client";
import { NetworkClient } from "./network";
import { checkEncryptedStorage } from "./storage-check";
import { address, formatUnits, hash, hexQuantity, quantity, safeNumber } from "./units";
import { button, card, detail, el, field, icon, info, input, pill } from "./ui";
import type { ActivityProof, ChainActivity, Network } from "./types";

// This entrypoint never imports Journal, opens a vault, or invokes a secret
// operation. The storage check creates/removes only its own unique public-data
// probe; address/transaction inputs remain in page memory.
const mount = document.getElementById("app")!;
const core = new CoreClient();
let networks: Network[] = [];
let ready = false;
let busy = false;
let generation = 0;
let active: AbortController | null = null;
const publicAddress = input("text", "", { id: "diagnostic-address", class: "mono", placeholder: "0x…", maxlength: 42, autocomplete: "off", autocapitalize: "none", spellcheck: "false", required: true });
const transactionHash = input("text", "", { id: "diagnostic-hash", class: "mono", placeholder: "0x…", maxlength: 66, autocomplete: "off", autocapitalize: "none", spellcheck: "false" });
const chainSelect = el("select", { id: "diagnostic-network", "aria-label": "Mainnet network" },
  el("option", { value: 8453 }, "Base mainnet"), el("option", { value: 1 }, "Ethereum mainnet"));
const balanceResult = el("div", { id: "diagnostic-balances", class: "form-stack", "aria-live": "polite" });
const receiptResult = el("div", { id: "diagnostic-receipt", class: "form-stack", "aria-live": "polite" });
const storageResult = el("div", { id: "diagnostic-storage", class: "form-stack", "aria-live": "polite" });
const status = el("p", { id: "diagnostic-status", class: "fine-print", role: "status", "aria-live": "polite" });
const balanceButton = button("Check balances", () => void checkBalances(), "primary wide", "refresh");
balanceButton.id = "diagnostic-check-balances";
const receiptButton = button("Verify transaction", () => void checkReceipt(), "secondary wide", "activity");
receiptButton.id = "diagnostic-check-transaction";
const storageButton = button("Check encrypted storage", () => void checkStorage(), "secondary wide", "shield");
storageButton.id = "diagnostic-check-storage";

function selectedNetwork(): Network {
  const network = networks.find(entry => entry.chain_id === Number(chainSelect.value));
  if (!network) throw new Error("The selected mainnet is unavailable.");
  return network;
}
function networkLabel(network: Network): string { return network.chain_id === 8453 ? "Base" : "Ethereum"; }
function controls(): void {
  for (const control of [publicAddress, transactionHash, chainSelect, balanceButton, receiptButton, storageButton]) control.disabled = !ready || busy;
}
async function checkStorage(): Promise<void> {
  if (!ready || busy) return;
  const token = ++generation;
  busy = true; controls(); storageResult.replaceChildren();
  status.textContent = "Checking encrypted storage with public test data…";
  try {
    await checkEncryptedStorage();
    if (generation !== token) return;
    storageResult.replaceChildren(card("Encrypted storage verified", "The browser saved and reopened a non-extractable encryption key and decrypted the public test data.",
      detail("Durable storage commit", "Passed"), detail("Key", "AES-256-GCM · non-extractable"),
      detail("Decryption after reopening", "Passed"), detail("Key export", "Blocked as required"),
      detail("Temporary test data", "Removed"), detail("Wallet data", "Not opened or modified")));
    status.textContent = "Local encryption check passed. Your wallet was not opened.";
  } catch (failure) {
    if (generation === token) { showFailure(storageResult, failure); status.textContent = "Encrypted storage check unsuccessful. Your wallet was not opened."; }
  } finally { if (generation === token) { busy = false; controls(); } }
}
function connectionDetails(client: NetworkClient): HTMLElement {
  const connection = client.connectionInfo();
  return el("div", {},
    detail("Provider", connection.provider ?? "No successful response"),
    detail("Successful transport", connection.transport === "relay" ? "Hosted relay" : connection.transport === "direct" ? "Direct RPC" : "Not established"),
    connection.endpoint ? detail("Endpoint", connection.endpoint, true) : null,
    detail("Failed read attempts", String(connection.failed_attempts)));
}
function showFailure(target: HTMLElement, failure: unknown, client?: NetworkClient): void {
  target.replaceChildren(info(failure instanceof Error ? failure.message : "This read-only check could not complete.", "warning"),
    ...(client ? [connectionDetails(client)] : []));
}
async function run(target: HTMLElement, label: string, task: (client: NetworkClient, owner: string, current: () => boolean) => Promise<void>): Promise<void> {
  if (!ready || busy) return;
  const token = ++generation;
  let client: NetworkClient | undefined;
  busy = true; controls(); status.textContent = label; target.replaceChildren();
  active = new AbortController();
  const current = () => generation === token && !active?.signal.aborted;
  try {
    const owner = address(publicAddress.value.trim());
    client = new NetworkClient(selectedNetwork(), undefined, { signal: active.signal });
    await task(client, owner, current);
    if (current()) status.textContent = "Read-only check complete. Results describe the observed chain state.";
  } catch (failure) {
    if (current()) { showFailure(target, failure, client); status.textContent = "Check unsuccessful. The error is shown below."; }
  } finally {
    if (generation === token) { active = null; busy = false; controls(); }
  }
}
async function checkBalances(): Promise<void> {
  await run(balanceResult, "Reading current balances…", async (client, owner, current) => {
    const snapshot = await client.balances(owner);
    if (!current()) return;
    balanceResult.replaceChildren(card("Balances verified", `${networkLabel(client.network)} mainnet · block ${snapshot.block_number.toLocaleString("en-US")}`,
      detail("Public address", snapshot.address, true),
      ...client.network.assets.map(asset => detail(asset.symbol, `${formatUnits(snapshot.balances[asset.symbol], asset.decimals)} ${asset.symbol}`)),
      detail("Observed", new Date(snapshot.observed_at * 1000).toISOString()),
      detail("Canonical block hash", snapshot.block_hash, true), connectionDetails(client)));
  });
}
function object(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === "object" && !Array.isArray(value); }
function activityDetails(item: ChainActivity): HTMLElement {
  const direction = item.direction === "received" ? "Received" : item.direction === "sent" ? "Sent" : "Self transfer";
  return el("section", { class: "fee-panel" }, el("h3", {}, `${direction} ${item.amount} ${item.asset}`),
    detail("Inclusion", item.inclusion === "finalized" ? "Finalized (provider report)" : "Included; awaiting finality"),
    detail("From", item.from, true), detail("To", item.to, true),
    item.asset_contract ? detail("Token contract", item.asset_contract, true) : null,
    detail("Block", item.block_number.toLocaleString("en-US")));
}
async function checkReceipt(): Promise<void> {
  await run(receiptResult, "Reading and validating the transaction receipt…", async (client, owner, current) => {
    const requested = transactionHash.value.trim();
    if (!/^0x[0-9a-fA-F]{64}$/.test(requested)) throw new Error("Enter the full 0x transaction hash (64 hexadecimal characters after 0x).");
    const txHash = hash(requested);
    await client.verifyChain();
    const [transaction, receipt] = await Promise.all([
      client.read("eth_getTransactionByHash", [txHash]), client.read("eth_getTransactionReceipt", [txHash]),
    ]);
    if (!current()) return;
    if (transaction === null && receipt === null) {
      receiptResult.replaceChildren(info(`The provider did not find this hash on ${networkLabel(client.network)} mainnet. This does not prove that the transfer failed.`), connectionDetails(client));
      return;
    }
    if (!object(transaction) || hash(transaction.hash) !== txHash) throw new Error("The provider returned inconsistent transaction evidence.");
    if (receipt === null) {
      receiptResult.replaceChildren(info("No receipt is available yet. A confirmed transfer cannot be established from this observation."), connectionDetails(client));
      return;
    }
    if (!object(receipt) || hash(receipt.transactionHash) !== txHash) throw new Error("The provider returned inconsistent receipt evidence.");
    if (transaction.blockHash === null || transaction.blockNumber === null) throw new Error("The transaction and receipt disagree about block inclusion. Retry after the provider reconciles them.");
    const height = safeNumber(quantity(receipt.blockNumber));
    const block = await client.block(hexQuantity(height));
    if (hash(receipt.blockHash) !== hash(block.hash)) throw new Error("The receipt is not in the currently observed canonical block.");
    let finalizedHeight = 0;
    let finalityWarning = "";
    try { finalizedHeight = safeNumber(quantity((await client.block("finalized")).number)); }
    catch { finalityWarning = "Finality could not be read. A successful transfer is shown as included without a finality claim."; }
    if (!current()) return;
    const proof: ActivityProof = { transaction, receipt, canonical_block: block, finalized_height: finalizedHeight };
    const validation = await core.validateActivity(client.network.chain_id, owner, proof);
    await client.assertCanonical(block);
    if (!current()) return;
    if (quantity(receipt.status) === 0n) {
      receiptResult.replaceChildren(info("This transaction reverted. It did not complete a transfer; the sender may still have paid gas.", "warning"), connectionDetails(client));
      return;
    }
    if (!validation.activities.length) {
      receiptResult.replaceChildren(info("No supported direct ETH or native USDC/EURC transfer involving this address was found in the validated receipt. Internal ETH transfers require separate indexing."), connectionDetails(client));
      return;
    }
    receiptResult.replaceChildren(card("Transaction verified", "Checked by the wallet’s shared local Rust validator.",
      detail("Network", `${networkLabel(client.network)} mainnet`), detail("Public address", owner, true),
      detail("Transaction", el("a", { href: `${client.network.explorer_url}/tx/${txHash}`, target: "_blank", rel: "noopener noreferrer", class: "mono break" }, txHash)),
      el("div", { class: "form-stack" }, ...validation.activities.map(activityDetails)),
      finalityWarning ? info(finalityWarning) : null,
      connectionDetails(client)));
  });
}
function clearResults(): void { if (!busy) { balanceResult.replaceChildren(); receiptResult.replaceChildren(); status.textContent = "Ready for a public address."; } }
publicAddress.addEventListener("input", clearResults);
chainSelect.addEventListener("change", clearResults);
transactionHash.addEventListener("input", () => { if (!busy) receiptResult.replaceChildren(); });
window.addEventListener("pagehide", () => { generation++; active?.abort(); core.cancelAll(); publicAddress.value = ""; transactionHash.value = ""; });

async function start(): Promise<void> {
  try {
    if (!isSecureContext || !globalThis.Worker || !globalThis.crypto?.getRandomValues || !globalThis.WebAssembly) throw new Error("Open diagnostics over HTTPS in a browser with WebAssembly and Web Workers enabled.");
    if (window.top !== window.self) throw new Error("Open wallet diagnostics in its own browser tab.");
    const registry = await core.registry();
    if (registry.networks.length !== 2 || !registry.networks.some(entry => entry.chain_id === 1) || !registry.networks.some(entry => entry.chain_id === 8453)) throw new Error("The supported mainnet registry is invalid.");
    networks = registry.networks;
    mount.replaceChildren(
      el("header", { class: "topbar" }, el("a", { href: "/", class: "brand" }, el("span", { class: "brand-symbol" }, icon("wallet")), el("span", {}, "Local Wallet")), el("span", { class: "header-caption" }, "Connection diagnostics"), pill("Read only", "subtle")),
      el("main", { id: "main", class: "app-main" }, el("div", { class: "narrow wide-narrow form-stack" },
        el("div", { class: "page-heading" }, el("div", {}, el("h1", {}, "Check public chain data"), el("p", {}, "Use a public wallet address. No wallet password or recovery words are needed."))),
        card("Connection and balances", "Uses the same mainnet client and transport as your wallet.", el("div", { class: "form-stack" },
          field("Network", chainSelect), field("Public wallet address", publicAddress), balanceButton)),
        balanceResult,
        card("Verify a transfer", "Optional: check a specific transaction against its receipt and canonical block.", el("div", { class: "form-stack" }, field("Transaction hash", transactionHash), receiptButton)),
        receiptResult, status,
        card("Encrypted browser storage", "Checks this browser using a separate temporary database and public test data. Does not open or modify your wallet. No address, password or recovery words are needed.", storageButton),
        storageResult,
        el("p", { class: "fine-print" }, "Local Wallet · v0.5.0. These checks use public blockchain providers. Canonical-block and receipt validation do not constitute independent consensus verification. Results are not saved to your wallet history."),
        el("a", { href: "/", class: "button text" }, icon("back"), "Back to wallet"))));
    ready = true; controls(); status.textContent = "Ready for a public address.";
  } catch (failure) {
    core.cancelAll();
    mount.replaceChildren(el("main", { id: "main", class: "startup-error" }, el("h1", {}, "Diagnostics could not open"), info(failure instanceof Error ? failure.message : "The read-only checker could not initialize.", "warning")));
  }
}
void start();
