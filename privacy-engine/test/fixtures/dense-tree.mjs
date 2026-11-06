/** Real-capacity public-tree fixtures. Builds exact dense levels bottom-up
 * with the pinned Poseidon hash. Repeated filler subtrees are memoized only in
 * this test helper; no production capacity or membership check is replaced. */
import { keccak256, toUtf8Bytes } from 'ethers';
import { MerkleForest } from '../../src/merkle-forest.mjs';
import { FIELD, poseidon } from '../../src/protocol.mjs';

const zeros = [BigInt(keccak256(toUtf8Bytes('Railgun'))) % FIELD];
for (let i = 0; i < 16; i++) zeros.push(poseidon([zeros[i], zeros[i]]));

export function denseTree(tree, prefix, count = 65536, filler = 999n) {
  if (!Number.isSafeInteger(count) || count < prefix.length || count < 1 || count > 65536) throw Error('FIXTURE');
  const leaves = Array(count).fill(BigInt(filler));
  for (let i = 0; i < prefix.length; i++) leaves[i] = BigInt(prefix[i]);
  if (leaves.some(leaf => leaf <= 0n || leaf >= FIELD)) throw Error('FIXTURE');
  const forest = new MerkleForest(), state = forest.tree(tree), memo = new Map();
  state.count = count;
  leaves.forEach((leaf, position) => state.levels[0].set(position, leaf));
  let width = count;
  for (let level = 0; level < 16; level++) {
    for (let position = 0; position < width; position += 2) {
      const left = state.levels[level].get(position) ?? zeros[level];
      const right = state.levels[level].get(position + 1) ?? zeros[level];
      const id = `${left}:${right}`;
      let value = memo.get(id);
      if (value === undefined) { value = poseidon([left, right]); memo.set(id, value); }
      state.levels[level + 1].set(position >> 1, value);
    }
    width = Math.ceil(width / 2);
  }
  return forest;
}
