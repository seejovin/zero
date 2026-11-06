import { test } from "node:test";
import assert from "node:assert/strict";
import { assetRefKey, totalUsdc, validateAssetRef, type ChainUsdcBalance } from "../src/portfolio";

test("asset selection distinguishes every supported asset on both chains and rejects incomplete or unsupported selections", () => {
  const keys = new Set<string>();
  for (const chainId of [8453, 1] as const) for (const symbol of ["USDC", "EURC", "ETH"] as const) {
    const ref = validateAssetRef({ chainId, symbol });
    assert.deepEqual(ref, { chainId, symbol });
    keys.add(assetRefKey(ref));
  }
  assert.equal(keys.size, 6);
  assert.equal(assetRefKey({ chainId: 8453, symbol: "USDC" }), "8453:USDC");
  for (const invalid of [null, [], "USDC", { symbol: "USDC" }, { chainId: 8453 }, { chainId: "8453", symbol: "USDC" }, { chainId: 10, symbol: "USDC" }, { chainId: 8453, symbol: "USDC.e" }]) {
    assert.throws(() => validateAssetRef(invalid), /supported asset and network/);
  }
});

test("total USDC sums exact quantities from both networks independent of row order", () => {
  assert.equal(totalUsdc([{ chainId: 8453, amount: "0.01" }, { chainId: 1, amount: "0" }]), "0.01");
  assert.equal(totalUsdc([{ chainId: 1, amount: "1.000001" }, { chainId: 8453, amount: "2.999999" }]), "4");
  assert.equal(totalUsdc([{ chainId: 8453, amount: "0.000000" }, { chainId: 1, amount: "0" }]), "0");
  assert.equal(totalUsdc([{ chainId: 1, amount: "9007199254.740993" }, { chainId: 8453, amount: "0.000001" }]), "9007199254.740994");
});

test("missing, unavailable, duplicated or unsupported network coverage never becomes a zero-filled total", () => {
  const incomplete: unknown[] = [
    [], [{ chainId: 8453, amount: "0.01" }], [{ chainId: 1, amount: "0.01" }],
    [{ chainId: 8453, amount: "0.01" }, { chainId: 1, amount: null }],
    [{ chainId: 8453, amount: null }, { chainId: 1, amount: "0" }],
    [{ chainId: 8453, amount: "0.01" }, { chainId: 8453, amount: "0.02" }],
    [{ chainId: 1, amount: "0.01" }, { chainId: 1, amount: "0.02" }],
    [{ chainId: 8453, amount: "0.01" }, { chainId: 10, amount: "0.02" }],
    [{ chainId: "8453", amount: "0.01" }, { chainId: 1, amount: "0.02" }],
    [{ chainId: 8453, amount: "0.01" }, { chainId: 1, amount: "0.02" }, { chainId: 1, amount: "0" }],
    [null, { chainId: 1, amount: "0" }], null,
  ];
  for (const entries of incomplete) assert.equal(totalUsdc(entries as ChainUsdcBalance[]), null);
});

test("malformed quantities and precision beyond USDC's six decimals are unavailable rather than rounded", () => {
  for (const amount of ["", " ", "-1", "01", "0.0000001", "1.", ".1", "1e3", "NaN", "Infinity", " 0.01", "0.01 ", "0.01\n", "0.01\r\n", "0.01\u2028", "9".repeat(91), 0, undefined]) {
    assert.equal(totalUsdc([{ chainId: 8453, amount }, { chainId: 1, amount: "0" }] as ChainUsdcBalance[]), null);
  }
});

test("maximum chain balances sum without numeric overflow; out-of-range chain quantities are rejected", () => {
  const maximum = "115792089237316195423570985008687907853269984665640564039457584007913129.639935";
  assert.equal(totalUsdc([{ chainId: 8453, amount: maximum }, { chainId: 1, amount: maximum }]),
    "231584178474632390847141970017375815706539969331281128078915168015826259.27987");
  assert.equal(totalUsdc([{ chainId: 8453, amount: "115792089237316195423570985008687907853269984665640564039457584007913129.639936" }, { chainId: 1, amount: "0" }]), null);
});
