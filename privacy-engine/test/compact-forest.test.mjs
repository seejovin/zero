/** Differential proof/path tests over actual pinned protocol hash primitives.
 * Fixtures are local synthetic pool history; no chain root is claimed valid. */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CompactForest } from '../src/compact-forest.mjs';
import { MerkleForest } from '../src/merkle-forest.mjs';
import { denseTree } from './fixtures/dense-tree.mjs';
import {
  localKeys, makeTransfer, Transaction, walletFacade, NO_ADAPT, VERSION, CHAIN,
  TOKEN, hex32, getTokenDataERC20,
} from '../src/protocol.mjs';

const leaves = (count, offset = 1) => Array.from({ length: count }, (_, i) => hex32(offset + i));
function completeActiveFixture(compact, full, tree) {
  const prefix = compact.active.export()[0].leaves;
  // This fixture represents a fully scanned, real-capacity public tree. It
  // avoids hashing identical filler paths 65,536 times in every unit test.
  compact.active = denseTree(tree, prefix);
  full.trees.set(tree, denseTree(tree, prefix).trees.get(tree));
}
function build() {
  const full = new MerkleForest(), compact = new CompactForest();
  for (let tree = 0; tree < 4; tree++) {
    const data = leaves(31 + tree, 100 * tree + 1);
    for (let start = 0; start < data.length;) {
      const end = Math.min(start + 7, data.length), chunk = data.slice(start, end);
      full.append(tree, start, chunk); compact.append(tree, start, chunk);
      if (start === 0) { compact.retain(tree, 0); compact.retain(tree, 3); }
      assert.equal(compact.root(tree), full.root(tree));
      for (const owned of [0, 3]) assert.deepEqual(compact.proof(tree, owned), full.proof(tree, owned));
      start = end;
    }
    if (tree < 3) completeActiveFixture(compact, full, tree);
    compact.retain(tree, full.count(tree) - 1);
  }
  return { full, compact };
}

test('dense capacity fixtures reproduce ordinary append roots and paths', () => {
  for (const count of [1, 2, 3, 7, 16, 31, 65]) {
    const prefix = leaves(Math.min(5, count)), expected = new MerkleForest();
    expected.append(0, 0, [...prefix, ...Array(count - prefix.length).fill(hex32(999))]);
    const dense = denseTree(0, prefix, count);
    assert.equal(dense.count(0), expected.count(0));
    assert.equal(dense.root(0), expected.root(0));
    for (const position of [0, Math.floor(count / 2), count - 1]) {
      assert.deepEqual(dense.proof(0, position), expected.proof(0, position));
      assert.equal(dense.leaf(0, position), expected.leaf(0, position));
    }
  }
});

test('archival retains exact latest paths while discarding closed unowned public leaves', () => {
  const { full, compact } = build();
  assert.equal(compact.activeTree, 3);
  for (let tree = 0; tree < 4; tree++) {
    assert.equal(compact.root(tree), full.root(tree));
    assert.equal(compact.count(tree), full.count(tree));
    for (const position of [0, 3, full.count(tree) - 1]) {
      const proof = compact.proof(tree, position);
      assert.deepEqual(proof, full.proof(tree, position));
      assert.equal(compact.leaf(tree, position), full.leaf(tree, position));
      assert.equal(MerkleForest.checkProof(compact.leaf(tree, position), position, proof, full.root(tree)), true);
    }
  }
  assert.throws(() => compact.leaf(0, 1), /MISSING_NOTE/);
  assert.throws(() => compact.retain(0, 1), /MISSING_NOTE/);
  const snapshot = compact.export();
  assert.equal(snapshot.trees.filter(tree => tree.kind === 'active').length, 1);
  for (const old of snapshot.trees.slice(0, -1)) {
    assert.equal(old.leaves, undefined); assert.equal(old.witnesses.length, 3);
    assert.equal(old.witnesses.every(w => w.siblings.length === 16), true);
  }
  assert.equal(compact.active.trees.size, 1);
  assert.deepEqual([...compact.active.trees.keys()], [3]);
});

test('archived witnesses and current retained positions survive strict snapshot restore', () => {
  const { full, compact } = build(), encoded = compact.export(), restored = CompactForest.import(encoded);
  assert.deepEqual(restored.export(), encoded);
  const additional = leaves(11, 999);
  full.append(3, 34, additional); restored.append(3, 34, additional);
  completeActiveFixture(restored, full, 3);
  full.append(4, 0, leaves(1, 2000)); restored.append(4, 0, leaves(1, 2000));
  for (const position of [0, 3, 33]) assert.deepEqual(restored.proof(3, position), full.proof(3, position));
  // Exported/provided witness arrays must not alias archived authoritative data.
  const witness = restored.proof(0, 0), expected = [...witness]; witness[0] = hex32(1000);
  const snapshot = restored.export(); snapshot.trees[0].witnesses[0].siblings[0] = hex32(1001);
  assert.deepEqual(restored.proof(0, 0), expected);
});

test('closed-tree edits, skipped trees and invalid batches fail without partial mutation', () => {
  const { compact } = build(), before = compact.export();
  for (const operation of [
    () => compact.append(0, 31, leaves(1)),
    () => compact.append(5, 0, leaves(1)),
    () => compact.append(4, 1, leaves(1)),
    () => compact.append(4, 0, leaves(1)),
    () => compact.append(3, 34, [hex32(40), 'not-a-hash']),
    () => compact.append(3, 34, [true]),
    () => compact.append(4, 0, [hex32(40), hex32(0)]),
  ]) {
    assert.throws(operation); assert.deepEqual(compact.export(), before);
  }
  assert.throws(() => new CompactForest().append(1, 0, leaves(1)), /INCOMPLETE_TREE/);
});

test('a rollover must exceed capacity for the exact complete insertion batch', () => {
  const forest = new CompactForest();
  forest.append(0, 0, leaves(3)); forest.retain(0, 1);
  forest.active = denseTree(0, leaves(3), 65534);
  const before = forest.export();
  assert.throws(() => forest.append(1, 0, leaves(2, 10)), /INCOMPLETE_TREE/);
  assert.deepEqual(forest.export(), before, 'a batch that exactly fits cannot skip the old tail');
  const expected = forest.active;
  forest.append(1, 0, leaves(3, 10));
  assert.equal(forest.count(0), 65534);
  assert.equal(forest.root(0), expected.root(0));
  assert.deepEqual(forest.proof(0, 1), expected.proof(0, 1));
  assert.equal(forest.count(1), 3);
  assert.deepEqual(CompactForest.import(forest.export()).export(), forest.export());
  const impossible = forest.export(); impossible.trees[1].leaves.pop();
  assert.throws(() => CompactForest.import(impossible), /INVALID_STATE/);
});

test('tampered archived witnesses, ambiguous positions and malformed snapshots are rejected', () => {
  const { compact } = build(), baseline = compact.export();
  const changes = [
    s => { s.trees[0].root = hex32(999); },
    s => { s.trees[0].witnesses[0].leaf = hex32(999); },
    s => { s.trees[0].witnesses[0].siblings[3] = hex32(999); },
    s => { s.trees[0].witnesses[0].position = 1; },
    s => { s.trees[0].witnesses.push(s.trees[0].witnesses[0]); },
    s => { s.trees[0].count = 3; },
    s => { s.trees[0].leaves = []; },
    s => { s.trees[3].retained_positions.push(3); },
    s => { s.trees[3].retained_positions = [3, 0]; },
    s => { s.trees[3].leaves[0] = hex32(0); },
    s => { s.active_tree = 2; },
    s => { s.trees.splice(1, 1); },
    s => { s.schema_version = 0; },
    s => { s.trees.shift(); },
  ];
  for (const change of changes) {
    const changed = structuredClone(baseline); change(changed);
    assert.throws(() => CompactForest.import(changed));
  }
  const empty = new CompactForest(); assert.deepEqual(CompactForest.import(empty.export()).export(), empty.export());
  assert.equal(empty.root(0), new MerkleForest().root(0));
});

test('legacy full-tree migration retains selected leaf positions, including duplicate commitments', () => {
  const full = new MerkleForest();
  full.append(0, 0, [hex32(1), hex32(1), hex32(2)]);
  const references = [{ tree: 0, position: 0 }, { tree: 0, position: 1 }];
  const compact = CompactForest.importLegacy(full.export(), references);
  for (const { tree, position } of references) {
    assert.equal(compact.leaf(tree, position), full.leaf(tree, position));
    assert.deepEqual(compact.proof(tree, position), full.proof(tree, position));
  }
  assert.equal(compact.export().trees[0].retained_positions.length, 2);
  assert.throws(() => CompactForest.importLegacy(full.export(), [...references, references[0]]));
  assert.throws(() => CompactForest.importLegacy(full.export(), [{ tree: 2, position: 0 }]));
  assert.throws(() => CompactForest.importLegacy(full.export(), [{ tree: 0, position: 50 }]));
  assert.throws(() => CompactForest.importLegacy([{ tree: 1, leaves: leaves(1) }], [{ tree: 1, position: 0 }]));
  assert.throws(() => CompactForest.importLegacy([...full.export(), { tree: 1, leaves: leaves(8) }], references), /INCOMPLETE_TREE/);
  assert.deepEqual(CompactForest.importLegacy([{ tree: 0, leaves: [] }], []).export(), new CompactForest().export());
});

test('genuine SDK requests use identical private Merkle witnesses from archived input trees', async () => {
  const own = await localKeys('61'.repeat(32), '62'.repeat(32));
  const input = makeTransfer(own.address, own, 100n), output = makeTransfer(own.address, own, 100n);
  const full = new MerkleForest(), compact = new CompactForest();
  const initial = [hex32(1), hex32(input.hash), hex32(2)];
  full.append(0, 0, initial); compact.append(0, 0, initial); compact.retain(0, 1);
  full.append(0, 3, leaves(10, 3)); compact.append(0, 3, leaves(10, 3));
  completeActiveFixture(compact, full, 0);
  full.append(1, 0, leaves(4, 90)); compact.append(1, 0, leaves(4, 90));
  const tx = new Transaction(CHAIN, getTokenDataERC20(TOKEN), 0, [{ note: input, position: 1 }], [output], NO_ADAPT);
  const a = await tx.generateTransactionRequest(walletFacade(own, full), VERSION, '', { minGasPrice: 0n });
  const b = await tx.generateTransactionRequest(walletFacade(own, compact), VERSION, '', { minGasPrice: 0n });
  // Randomized output ciphertext differs between requests. Input ownership,
  // value, membership paths and publicly committed root/nullifiers must agree.
  assert.deepEqual(b.privateInputs.pathElements, a.privateInputs.pathElements);
  assert.deepEqual(b.privateInputs.leavesIndices, a.privateInputs.leavesIndices);
  assert.deepEqual(b.privateInputs.valueIn, a.privateInputs.valueIn);
  assert.equal(b.publicInputs.merkleRoot, a.publicInputs.merkleRoot);
  assert.deepEqual(b.publicInputs.nullifiers, a.publicInputs.nullifiers);
  own.destroy();
});
