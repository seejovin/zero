import { test } from "node:test";
import assert from "node:assert/strict";
import { parseStrictJson } from "../src/strict-json";

test("strict JSON preserves valid JSON semantics and safely treats prototype-like names", () => {
  const value = parseStrictJson('{"x":[true,false,null,-1.5e2,"a\\\"b"],"__proto__":{"admin":true}}') as any;
  assert.deepEqual(value.x, [true, false, null, -150, 'a"b']);
  assert.equal(Object.getPrototypeOf(value), null);
  assert.equal(({} as any).admin, undefined);
});
test("conflicting receipt fields and escaped duplicate keys are rejected before normalization", () => {
  for (const input of ['{"status":"0x0","status":"0x1"}', '{"result":{"transactionHash":"one","transactionHash":"two"}}', '{"status":0,"st\\u0061tus":1}', '{"x":{"a":1,"a":1}}']) assert.throws(() => parseStrictJson(input));
});
test("malformed syntax, excessive depth/nodes, and unsafe JSON numbers fail closed", () => {
  for (const input of ['{"a":1,}', '[1,]', '{"x":01}', 'true false', '"\\x01"', '"unterminated', '1e999', '9007199254740993', '{"a":"\n"}']) assert.throws(() => parseStrictJson(input));
  assert.throws(() => parseStrictJson("[".repeat(34) + "0" + "]".repeat(34)));
  assert.throws(() => parseStrictJson("[" + "0,".repeat(50_001) + "0]"));
  assert.throws(() => parseStrictJson(" ".repeat(1_048_577)));
});
