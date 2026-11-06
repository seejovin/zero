import { test } from "node:test";
import assert from "node:assert/strict";
import { accountKey, formatUnits, quantity, receiveUri, units } from "../src/units";

test("amount parsing uses exact integers and round-trips all supported decimals", () => {
  assert.equal(units("9007199254.740993", 6), 9007199254740993n);
  assert.equal(formatUnits(9007199254740993n, 6), "9007199254.740993");
  assert.equal(units("0.000000000000000001", 18), 1n);
  assert.equal(formatUnits(1n, 18), "0.000000000000000001");
  assert.equal(formatUnits(1_250_000n, 6), "1.25");
  for (let decimals = 0; decimals <= 18; decimals++) {
    for (const value of [0n, 1n, 17n, 999999n, 2n ** 128n, (1n << 256n) - 1n]) assert.equal(units(formatUnits(value, decimals), decimals), value);
  }
});
test("ambiguous, negative, overprecise, nonfinite, and overflowing amounts are rejected", () => {
  for (const value of ["", ".1", "1.", "01", "1e3", "1,000", "-1", "+1", " 1", "1 ", "NaN", "Infinity", "0.0000001", (1n << 256n).toString()]) assert.throws(() => units(value, 6));
});
test("RPC quantities must use canonical hexadecimal encoding", () => {
  assert.equal(quantity("0x0"), 0n); assert.equal(quantity("0xff"), 255n);
  for (const value of [0, "0x00", "0x", "-1", "0X1", "0xg", `0x1${"0".repeat(64)}`]) assert.throws(() => quantity(value));
});
test("account reservation and receive URI are network-specific", () => {
  const address = "0x1111111111111111111111111111111111111111";
  assert.notEqual(accountKey(1, address), accountKey(8453, address));
  assert.throws(() => accountKey(11155111 as never, address));
  const network = { chain_id: 8453, assets: [{ symbol: "USDC", decimals: 6, contract: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913" }] } as never;
  assert.equal(receiveUri(network, address, "USDC"), `ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913@8453/transfer?address=${address}`);
});
