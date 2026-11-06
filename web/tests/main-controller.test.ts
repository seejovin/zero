import { test } from "node:test";
import assert from "node:assert/strict";
import { activity, CONTROLLER_RECIPIENT, deferred, freshBalance, openController, payment, quote, settle } from "./controller-fixture";
import type { ChainId, FeePreview } from "../src/types";

type App = Awaited<ReturnType<typeof openController>>;
function view(app: App, chain: ChainId) { return app.inspect().views.find((item: any) => item.chainId === chain)!; }
function fillDraft(app: App, amount = "0.01") {
  const recipient = app.document.getElementById("send-recipient")!, quantity = app.document.getElementById("send-amount")!;
  recipient.value = CONTROLLER_RECIPIENT; recipient.dispatchEvent({ type: "input" });
  quantity.value = amount; quantity.dispatchEvent({ type: "input" });
  return { recipient, quantity };
}
function submit(app: App) { const form = app.mount.querySelector("form"); assert.ok(form); form.dispatchEvent({ type: "submit" }); }

test("controller refreshes both chains independently; one failure preserves the other balance without inventing a total", async () => {
  const base = deferred<any>(), ethereum = deferred<any>();
  const app = await openController(fixture => { fixture.methods.balanceStatus = chain => chain === 8453 ? base.promise : ethereum.promise; });
  app.controller.refreshWallet(true);
  assert.deepEqual(app.fixture.balanceCalls, [8453, 1]);
  base.resolve(freshBalance(8453, "10000"));
  await settle(() => view(app, 8453).state === "live");
  assert.equal(view(app, 8453).balances.USDC, "0.01");
  assert.equal(view(app, 1).balances.USDC, null);
  assert.equal(view(app, 1).state, "loading");
  ethereum.reject(new Error("Public fixture connection failure"));
  await settle(() => view(app, 1).state === "unavailable");
  assert.equal(view(app, 8453).state, "live");
  assert.equal(app.mount.querySelector(".hero-quantity")!.textContent, "—");
  assert.match(app.mount.querySelector('[data-asset-key="8453:USDC"]')!.textContent, /0\.01/);
  assert.match(app.mount.querySelector('[data-asset-key="1:USDC"]')!.textContent, /Unavailable/);
});

test("a late Base fee or balance cannot overwrite an Ethereum send selection or reconstruct its edited inputs", async () => {
  const oldFee = deferred<FeePreview>();
  const app = await openController(fixture => {
    fixture.methods.preview = async (chain, draft) => chain === 8453 ? oldFee.promise : quote(chain, draft.asset);
  });
  app.controller.beginTransfer("send", { chainId: 8453, symbol: "USDC" });
  fillDraft(app);
  const oldRequest = app.controller.estimateFee();
  app.controller.beginTransfer("send", { chainId: 1, symbol: "USDC" });
  const controls = fillDraft(app, "0.02"); controls.recipient.focus();
  await app.controller.estimateFee();
  assert.equal(app.inspect().preview.chain_id, 1);
  oldFee.resolve(quote(8453)); await oldRequest;
  app.fixture.methods.balanceStatus = async chain => freshBalance(chain, chain === 8453 ? "70000" : "20000");
  await app.controller.refreshBalances(8453);
  assert.equal(app.inspect().chain, 1);
  assert.equal(app.inspect().preview.chain_id, 1);
  assert.equal(app.inspect().sendDraft.amount, "0.02");
  assert.strictEqual(app.document.getElementById("send-recipient"), controls.recipient);
  assert.strictEqual(app.document.getElementById("send-amount"), controls.quantity);
  assert.strictEqual(app.document.activeElement, controls.recipient);
  assert.equal(controls.recipient.value, CONTROLLER_RECIPIENT);
  assert.equal(view(app, 8453).balances.USDC, "0.07");
});

test("an old balance response cannot repopulate a locked or reopened session", async () => {
  const pending = deferred<any>();
  const app = await openController(fixture => { fixture.methods.balanceStatus = () => pending.promise; });
  const read = app.controller.refreshBalances(8453), oldEpoch = app.inspect().epoch;
  app.controller.lock();
  assert.equal(app.inspect().unlocked, false);
  assert.equal(app.inspect().epoch, oldEpoch + 1);
  await app.controller.resumeFromStorage(); await app.idle();
  assert.equal(app.inspect().unlocked, true);
  pending.resolve(freshBalance(8453, "999999")); await read;
  assert.equal(app.fixture.writes.length, 0);
  assert.equal(view(app, 8453).balances.USDC, null);
  assert.equal(view(app, 1).balances.USDC, null);
});

test("deposit selection binds the QR to the chosen asset and chain while automatic reads preserve that screen", async () => {
  const app = await openController();
  app.controller.beginTransfer("deposit");
  assert.equal(app.inspect().route, "choose-deposit");
  app.document.getElementById("asset-picker")!.querySelector('[data-asset-key="1:USDC"]')!.click();
  assert.equal(app.inspect().route, "receive");
  assert.equal(app.inspect().chain, 1);
  assert.match(app.qr.at(-1)!, /^ethereum:0x1{40}@1\/transfer\?address=/);
  const canvas = app.mount.querySelector("canvas"), count = app.qr.length;
  await app.controller.refreshBalances(8453);
  assert.strictEqual(app.mount.querySelector("canvas"), canvas);
  assert.equal(app.qr.length, count);
  app.controller.beginTransfer("deposit", { chainId: 8453, symbol: "EURC" });
  assert.match(app.qr.at(-1)!, /^ethereum:0x4{40}@8453\/transfer\?address=/);
  assert.match(app.mount.textContent, /EURC on Base/);
});

test("merged history deduplicates only matching chain, hash and asset; pending records block only their own network", async () => {
  const hash = "0x" + "c".repeat(64);
  const app = await openController(fixture => {
    fixture.records.set("saved-base", payment(8453, "saved-base", "pending", hash));
    fixture.activities.set(8453, [activity(8453, hash, "sent"), activity(8453)]);
    fixture.activities.set(1, [activity(1, hash, "sent"), activity(1)]);
  });
  await settle(() => app.controller.displayActivities().length === 4);
  const rows = app.controller.displayActivities();
  assert.equal(rows.filter((row: any) => row.chainId === 8453 && row.direction === "outgoing").length, 1);
  assert.equal(rows.filter((row: any) => row.chainId === 1 && row.direction === "outgoing").length, 1);
  rows.find((row: any) => row.chainId === 1 && row.direction === "incoming").onOpen();
  assert.equal(app.inspect().chain, 1);
  assert.equal(app.inspect().currentActivity.chain_id, 1);
  app.controller.beginTransfer("send", { chainId: 1, symbol: "USDC" });
  assert.ok(app.document.getElementById("send-form"));
  app.controller.beginTransfer("send", { chainId: 8453, symbol: "USDC" });
  assert.equal(app.document.getElementById("send-form"), null);
  assert.match(app.mount.textContent, /Continue your transfer/);
  await app.controller.openPayment("saved-base");
  assert.equal(app.inspect().chain, 8453);
  assert.equal(app.inspect().currentPayment.intent_id, "saved-base");
});

test("reviewed Base payment stays bound to Base through another chain's update and a competing selection", async () => {
  const releaseSign = deferred<void>(), otherBalance = deferred<any>();
  let enteredSign = false;
  const app = await openController(fixture => {
    fixture.methods.balanceStatus = () => otherBalance.promise;
    const sign = fixture.methods.sign;
    fixture.methods.sign = async (...args) => { enteredSign = true; await releaseSign.promise; return sign(...args); };
  });
  app.controller.beginTransfer("send", { chainId: 8453, symbol: "USDC" });
  fillDraft(app); await app.controller.estimateFee(); submit(app); await app.idle();
  assert.equal(app.inspect().route, "review");
  assert.equal(app.inspect().reviewed.request.chain_id, 8453);
  const ethereumRead = app.controller.refreshBalances(1);
  app.mount.querySelector('input[type="checkbox"]')!.checked = true;
  submit(app); await settle(() => enteredSign);
  app.controller.beginTransfer("send", { chainId: 1, symbol: "USDC" });
  assert.equal(app.inspect().chain, 8453);
  otherBalance.resolve(freshBalance(1, "420000")); await ethereumRead;
  assert.equal(app.inspect().reviewed.request.chain_id, 8453);
  releaseSign.resolve(); await app.idle();
  assert.deepEqual(app.fixture.signedCalls, [8453]);
  assert.equal(app.inspect().currentPayment.prepared.request.chain_id, 8453);
  assert.equal(app.inspect().route, "transaction");
  assert.equal(view(app, 1).balances.USDC, "0.42");
});

test("profile rename persists as literal display text across reopen without changing the account or transfer records", async () => {
  const app = await openController(fixture => { fixture.records.set("original", payment(1, "original", "cancelled")); });
  const original = JSON.stringify([...app.fixture.records.values()]);
  app.controller.navigate("rename");
  app.document.getElementById("wallet-name")!.value = "<b>Reserve</b>";
  app.document.getElementById("account-name")!.value = "Daily account";
  submit(app);
  assert.equal(app.inspect().route, "profile");
  assert.match(app.mount.textContent, /<b>Reserve<\/b>/);
  assert.equal(app.mount.querySelector("b"), null);
  assert.equal(app.storage.size, 1);
  app.controller.lock(); await app.controller.resumeFromStorage(); await app.idle();
  assert.match(app.mount.textContent, /<b>Reserve<\/b>/);
  assert.match(app.mount.textContent, /Daily account/);
  assert.equal(JSON.stringify([...app.fixture.records.values()]), original);
  assert.equal(app.fixture.writes.length, 0);
});
