/**
 * Public pool history with one mutable full tree and archived owned witnesses.
 * The caller must retain each decrypted owned leaf before advancing output
 * trees, and independently verify every observed root against canonical chain
 * state. A valid Merkle witness alone does not establish history completeness.
 */
import { MerkleForest } from './merkle-forest.mjs';
import { FIELD, hex32 } from './protocol.mjs';

const CAPACITY = 65536, DEPTH = 16, MAX_OWNED = 100000;
const HASH = /^0x[0-9a-f]{64}$/;
const fail = code => { throw Error(code); };
const integer = (n, maximum) => {
  if (!Number.isSafeInteger(n) || n < 0 || n > maximum) fail('INVALID_STATE');
  return n;
};
const fields = (value, keys) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).sort().join(',') !== [...keys].sort().join(',')) fail('INVALID_STATE');
};
const scalar = (value, nonzero = false) => {
  if (typeof value !== 'string' || !HASH.test(value) || BigInt(value) >= FIELD
      || (nonzero && BigInt(value) === 0n)) fail('INVALID_STATE');
  return value;
};
const leafHash = value => {
  if (!['bigint', 'number', 'string'].includes(typeof value)) fail('INVALID_COMMITMENT');
  if (typeof value === 'number' && !Number.isSafeInteger(value)) fail('INVALID_COMMITMENT');
  let integer;
  try { integer = BigInt(value); } catch { fail('INVALID_COMMITMENT'); }
  if (integer <= 0n || integer >= FIELD) fail('INVALID_COMMITMENT');
  return hex32(integer);
};
const EMPTY_ROOT = new MerkleForest().root(0);

/** No private note values, keys or recipients are stored here. */
export class CompactForest {
  constructor() {
    this.trees = new Map();
    this.activeTree = null;
    this.active = new MerkleForest();
    this.retained = new Set();
    this.ownedCount = 0;
  }

  append(tree, start, values) {
    integer(tree, 65535); integer(start, CAPACITY - 1);
    if (!Array.isArray(values) || !values.length || values.length > CAPACITY
        || start + values.length > CAPACITY) fail('INCOMPLETE_TREE');
    // Validate the complete batch before archival or any mutable tree append.
    const leaves = values.map(leafHash);
    // This candidate scans the fixed pool's complete deployment history. No
    // independently reviewed migrated-tree anchor is supplied or inferred.
    if (this.activeTree === null && tree !== 0) fail('INCOMPLETE_TREE');
    if (this.activeTree !== null && tree !== this.activeTree && tree !== this.activeTree + 1) {
      fail('INCOMPLETE_TREE');
    }
    if (tree !== this.activeTree) {
      if (start !== 0 || this.trees.has(tree)) fail('INCOMPLETE_TREE');
      if (this.activeTree !== null) {
        // The pool emits one complete insertion batch for each Shield or
        // Transact call, including all commitments in a multi-proof call.
        // Commitments.insertLeaves starts a new tree only when the entire
        // batch no longer fits; equality still belongs in the current tree.
        if (this.active.count(this.activeTree) + leaves.length <= CAPACITY) fail('INCOMPLETE_TREE');
        this.archiveActive();
      }
      this.activeTree = tree;
      this.active = new MerkleForest();
      this.retained = new Set();
      this.trees.set(tree, { kind: 'active' });
    } else if (start !== this.active.count(tree)) fail('INCOMPLETE_TREE');
    this.active.append(tree, start, leaves);
  }

  archiveActive() {
    const tree = this.activeTree;
    if (tree === null) return;
    const witnesses = new Map();
    for (const position of [...this.retained].sort((a, b) => a - b)) {
      witnesses.set(position, {
        position, leaf: this.active.leaf(tree, position), siblings: this.active.proof(tree, position),
      });
    }
    this.trees.set(tree, {
      kind: 'closed', count: this.active.count(tree), root: this.active.root(tree), witnesses,
    });
    // No reference to the old full tree is retained by this object.
  }

  retain(tree, position) {
    integer(tree, 65535); integer(position, CAPACITY - 1);
    if (tree === this.activeTree) {
      this.active.leaf(tree, position);
      if (!this.retained.has(position)) {
        if (this.ownedCount >= MAX_OWNED) fail('FOREST_CAPACITY_EXCEEDED');
        this.retained.add(position); this.ownedCount++;
      }
      return;
    }
    // An archived unowned leaf cannot be recovered from its root alone.
    if (!this.trees.get(tree)?.witnesses?.has(position)) fail('MISSING_NOTE');
  }

  root(tree) {
    integer(tree, 65535);
    if (tree === this.activeTree) return this.active.root(tree);
    const saved = this.trees.get(tree);
    if (saved?.kind === 'closed') return saved.root;
    if (this.activeTree === null && tree === 0) return EMPTY_ROOT;
    fail('MISSING_TREE');
  }

  count(tree) {
    integer(tree, 65535);
    if (tree === this.activeTree) return this.active.count(tree);
    const saved = this.trees.get(tree);
    if (saved?.kind === 'closed') return saved.count;
    if (this.activeTree === null && tree === 0) return 0;
    fail('MISSING_TREE');
  }

  leaf(tree, position) {
    integer(tree, 65535); integer(position, CAPACITY - 1);
    if (tree === this.activeTree) return this.active.leaf(tree, position);
    const witness = this.trees.get(tree)?.witnesses?.get(position);
    if (!witness) fail('MISSING_NOTE');
    return witness.leaf;
  }

  proof(tree, position) {
    integer(tree, 65535); integer(position, CAPACITY - 1);
    if (tree === this.activeTree) return this.active.proof(tree, position);
    const witness = this.trees.get(tree)?.witnesses?.get(position);
    if (!witness) fail('MISSING_NOTE');
    // A consumer must not mutate the archived copy used by future proofs.
    return [...witness.siblings];
  }

  export() {
    return {
      schema_version: 1, active_tree: this.activeTree,
      trees: [...this.trees].sort(([a], [b]) => a - b).map(([tree, saved]) => {
        if (tree === this.activeTree) return {
          tree, kind: 'active', leaves: this.active.export()[0].leaves,
          retained_positions: [...this.retained].sort((a, b) => a - b),
        };
        return {
          tree, kind: 'closed', count: saved.count, root: saved.root,
          witnesses: [...saved.witnesses.values()].map(w => ({
            position: w.position, leaf: w.leaf, siblings: [...w.siblings],
          })),
        };
      }),
    };
  }

  static import(snapshot) {
    fields(snapshot, ['schema_version', 'active_tree', 'trees']);
    if (snapshot.schema_version !== 1 || !Array.isArray(snapshot.trees)
        || snapshot.trees.length > CAPACITY) fail('INVALID_STATE');
    const forest = new CompactForest();
    if (snapshot.active_tree === null) {
      if (snapshot.trees.length !== 0) fail('INVALID_STATE');
      return forest;
    }
    integer(snapshot.active_tree, 65535);
    if (!snapshot.trees.length) fail('INVALID_STATE');
    let previous = null, previousCount = null;
    for (const saved of snapshot.trees) {
      integer(saved?.tree, 65535);
      if (previous === null && saved.tree !== 0) fail('INVALID_STATE');
      if (previous !== null && saved.tree !== previous + 1) fail('INVALID_STATE');
      previous = saved.tree;
      if (saved.kind === 'active') {
        fields(saved, ['tree', 'kind', 'leaves', 'retained_positions']);
        if (saved.tree !== snapshot.active_tree || saved !== snapshot.trees.at(-1)
            || !Array.isArray(saved.leaves) || !saved.leaves.length || saved.leaves.length > CAPACITY
            || !Array.isArray(saved.retained_positions) || saved.retained_positions.length > saved.leaves.length) fail('INVALID_STATE');
        saved.leaves.forEach(leaf => scalar(leaf, true));
        forest.activeTree = saved.tree;
        forest.active.append(saved.tree, 0, saved.leaves);
        forest.trees.set(saved.tree, { kind: 'active' });
        let last = -1;
        for (const position of saved.retained_positions) {
          integer(position, saved.leaves.length - 1);
          if (position <= last) fail('INVALID_STATE');
          last = position; forest.retain(saved.tree, position);
        }
      } else if (saved.kind === 'closed') {
        fields(saved, ['tree', 'kind', 'count', 'root', 'witnesses']);
        if (saved.tree >= snapshot.active_tree || !integer(saved.count, CAPACITY)
            || !Array.isArray(saved.witnesses) || saved.witnesses.length > saved.count) fail('INVALID_STATE');
        scalar(saved.root);
        const witnesses = new Map(); let last = -1;
        for (const witness of saved.witnesses) {
          fields(witness, ['position', 'leaf', 'siblings']);
          integer(witness.position, saved.count - 1);
          if (witness.position <= last || !Array.isArray(witness.siblings) || witness.siblings.length !== DEPTH) fail('INVALID_STATE');
          last = witness.position;
          scalar(witness.leaf, true); witness.siblings.forEach(sibling => scalar(sibling));
          if (!MerkleForest.checkProof(witness.leaf, witness.position, witness.siblings, saved.root)) fail('INVALID_STATE');
          if (++forest.ownedCount > MAX_OWNED) fail('FOREST_CAPACITY_EXCEEDED');
          witnesses.set(witness.position, { position: witness.position, leaf: witness.leaf, siblings: [...witness.siblings] });
        }
        forest.trees.set(saved.tree, { kind: 'closed', count: saved.count, root: saved.root, witnesses });
      } else fail('INVALID_STATE');
      // Snapshots omit individual insertion boundaries, so this is the
      // necessary count relation between complete consecutive trees. The
      // original live append validates the exact first batch at rollover.
      const count = saved.kind === 'active' ? saved.leaves.length : saved.count;
      if (previousCount !== null && previousCount + count <= CAPACITY) fail('INVALID_STATE');
      previousCount = count;
    }
    if (forest.activeTree !== snapshot.active_tree) fail('INVALID_STATE');
    return forest;
  }

  /** Upgrade an authenticated legacy full-forest snapshot. Owned positions must
   * be supplied before old trees are discarded and checked against restored
   * SDK note commitments/nullifiers by the caller afterward. */
  static importLegacy(trees, ownedPositions) {
    if (!Array.isArray(trees) || trees.length > CAPACITY || !Array.isArray(ownedPositions)
        || ownedPositions.length > MAX_OWNED) fail('INVALID_STATE');
    const keep = new Map(), seen = new Set();
    for (const entry of ownedPositions) {
      fields(entry, ['tree', 'position']); integer(entry.tree, 65535); integer(entry.position, CAPACITY - 1);
      const id = `${entry.tree}:${entry.position}`;
      if (seen.has(id)) fail('INVALID_STATE');
      seen.add(id);
      if (!keep.has(entry.tree)) keep.set(entry.tree, []);
      keep.get(entry.tree).push(entry.position);
    }
    const ordered = [...trees].sort((a, b) => a.tree - b.tree), forest = new CompactForest();
    if (ordered.length && ordered[0]?.tree !== 0) fail('INVALID_STATE');
    if (ordered.length === 1 && ordered[0]?.tree === 0
        && Array.isArray(ordered[0].leaves) && ordered[0].leaves.length === 0 && !keep.size) {
      fields(ordered[0], ['tree', 'leaves']);
      return forest;
    }
    for (const tree of ordered) {
      fields(tree, ['tree', 'leaves']); integer(tree.tree, 65535);
      if (!Array.isArray(tree.leaves) || !tree.leaves.length || tree.leaves.length > CAPACITY) fail('INVALID_STATE');
      tree.leaves.forEach(leaf => scalar(leaf, true));
      forest.append(tree.tree, 0, tree.leaves);
      for (const position of keep.get(tree.tree) ?? []) forest.retain(tree.tree, position);
      keep.delete(tree.tree);
    }
    if (keep.size) fail('INVALID_STATE');
    return forest;
  }
}
