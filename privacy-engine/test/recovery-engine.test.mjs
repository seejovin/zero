/** Offline canonical-adapter fixtures with genuine SDK encrypted notes and
 * transaction request construction. No fixture pretends to prove consensus,
 * verify a SNARK, or represent a funded mainnet wallet. */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sha256 } from 'ethers';
import {
  localKeys, makeTransfer, Transaction, walletFacade, NO_ADAPT, VERSION, CHAIN,
  hex32, getTokenDataERC20, TOKEN, feeRecipientCommitment, TransactNote,
} from '../src/protocol.mjs';
import { CANDIDATE, poolInterface } from '../src/candidate-calldata.mjs';
import { MerkleForest } from '../src/merkle-forest.mjs';
import { denseTree } from './fixtures/dense-tree.mjs';
import { LocalPrivateEngine } from '../src/local-engine.mjs';
import { captureRecoveryCutoff, scanRecovery, prepareRecoveryBatch } from '../src/recovery-engine.mjs';

const h = n => `0x${BigInt(n).toString(16).padStart(64, '0')}`;
const q = n => `0x${n.toString(16)}`;
const own = await localKeys('11'.repeat(32), '12'.repeat(32));
const payee = await localKeys('21'.repeat(32), '22'.repeat(32));
const domain = {
  protocol: CANDIDATE.protocol, chain_id: 42161, pool_contract: CANDIDATE.pool,
  token_contract: TOKEN, token_decimals: 6, verifier_digest: h(1),
};
const manifest = {
  domain, fee_recipient_address: payee.address, deployment_block: 0,
  circuits: { '01x03': {}, '02x03': {} },
};
const fee = {
  domain, schedule_id: h(2), fee_units: '7', fee_recipient_address: payee.address,
  fee_recipient_commitment: feeRecipientCommitment(payee.address),
  max_native_fee_wei: '1000000000000000', gas_limit: 1000000,
  valid_from: 1, valid_until: 9999999999,
};

function storage() {
  let saved = null, upload = null;
  return async (port, p) => {
    if (port === 'storage_write') {
      if (p.action === 'begin') {
        upload = { id: h(8), bytes: new Uint8Array(p.total_bytes), hash: p.sha256, offset: 0 };
        return { upload_id: upload.id, offset: 0 };
      }
      if (p.action === 'write') {
        assert.equal(p.upload_id, upload.id); assert.equal(p.offset, upload.offset);
        const chunk = Buffer.from(p.base64, 'base64');
        upload.bytes.set(chunk, p.offset); upload.offset += chunk.length;
        return { upload_id: upload.id, offset: upload.offset };
      }
      if (p.action === 'commit') {
        assert.equal(upload.offset, upload.bytes.length);
        assert.equal(sha256(upload.bytes).slice(2), upload.hash);
        saved = upload.bytes;
        return { committed: true, total_bytes: saved.length, sha256: upload.hash };
      }
      if (p.action === 'abort') { upload = null; return { aborted: true }; }
    }
    if (port === 'storage_read') return saved ? {
      exists: true, base64: Buffer.from(saved.subarray(p.offset, p.offset + p.length)).toString('base64'),
      offset: p.offset, total_bytes: saved.length, sha256: sha256(saved).slice(2),
    } : { exists: false, base64: null, offset: 0, total_bytes: 0, sha256: null };
    throw Error('Unexpected storage port');
  };
}

async function encryptedEvent(tree, start, notes, at, index = 0) {
  const dummyInput = makeTransfer(own.address, own, notes.reduce((sum, note) => sum + note.value, 0n));
  const dummyForest = new MerkleForest(); dummyForest.append(tree, 0, [hex32(dummyInput.hash)]);
  const tx = new Transaction(CHAIN, getTokenDataERC20(TOKEN), tree,
    [{ note: dummyInput, position: 0 }], notes, NO_ADAPT);
  const prepared = await tx.generateTransactionRequest(walletFacade(own, dummyForest), VERSION, '', { minGasPrice: 0n });
  const encoded = poolInterface.encodeEventLog('Transact', [
    tree, start, notes.map(note => hex32(note.hash)), prepared.boundParams.commitmentCiphertext,
  ]);
  return {
    address: CANDIDATE.pool, removed: false, blockNumber: q(at), blockHash: h(at),
    transactionHash: h(1000 + at), transactionIndex: '0x0', logIndex: q(index), ...encoded,
  };
}

function nullifierEvent(tree, nullifiers, at) {
  return {
    address: CANDIDATE.pool, removed: false, blockNumber: q(at), blockHash: h(at),
    transactionHash: h(1000 + at), transactionIndex: '0x0', logIndex: '0x0',
    ...poolInterface.encodeEventLog('Nullified', [tree, nullifiers]),
  };
}

async function fixture(logs = [], { latest = 10, finalized = 10 } = {}) {
  const local = storage(), calls = [], state = { latest, finalized, logs: [...logs], hashes: new Map(), omittedTransactions: new Set(), prefix: null };
  const block = number => ({ number, hash: state.hashes.get(number) ?? h(number) });
  const result = (p, values) => ({ schema_version: 1, chain_id: 42161, kind: p.kind, ...values });
  const baseForest = number => {
    if (state.prefix && number >= state.prefix.block) return copyForest(state.prefix.forest);
    return new MerkleForest();
  };
  const port = async (kind, p) => {
    if (kind !== 'rpc') return local(kind, p);
    calls.push(p);
    if (p.kind === 'tip') return result(p, { block: block(state.latest) });
    if (p.kind === 'head') return result(p, { block: block(state.finalized) });
    if (p.kind === 'block') return result(p, { block: block(p.number) });
    if (p.kind === 'block_logs') {
      assert(p.number <= state.finalized);
      return result(p, { block: block(p.number), logs: state.logs.filter(log => Number(log.blockNumber) === p.number) });
    }
    if (p.kind === 'checkpoint') {
      assert(p.number <= state.finalized);
      const forest = baseForest(p.number); let tree = Math.max(0, ...forest.trees.keys());
      for (const log of state.logs) {
        if (Number(log.blockNumber) > p.number) continue;
        const parsed = poolInterface.parseLog(log);
        if (parsed.name !== 'Transact') continue;
        tree = Number(parsed.args.treeNumber);
        forest.append(tree, Number(parsed.args.startPosition), [...parsed.args.hash]);
      }
      return result(p, { block: block(p.number), tree_number: tree,
        next_leaf_index: forest.count(tree), merkle_root: forest.root(tree) });
    }
    if (p.kind === 'logs') {
      // Match the real private pool endpoint's aligned-range privacy policy.
      const count = p.to_block - p.from_block + 1;
      const aligned = Number.isInteger(Math.log2(count)) && p.from_block % count === 0;
      const truncatedTip = p.to_block === state.finalized && Array.from({ length: 12 }, (_, n) => 2 ** n)
        .some(width => p.from_block % width === 0 && count <= width);
      if (count < 1 || count > 2048 || p.to_block > state.finalized || (!aligned && !truncatedTip)) {
        throw Error('INVALID_POOL_RANGE');
      }
      return result(p, {
        from_block: p.from_block, to_block: p.to_block, block_to_hash: block(p.to_block).hash,
        logs: state.logs.filter(log => Number(log.blockNumber) >= p.from_block && Number(log.blockNumber) <= p.to_block
          && !state.omittedTransactions.has(log.transactionHash)),
      });
    }
    if (p.kind === 'root') {
      const forest = baseForest(state.latest), roots = new Set([forest.root(p.tree_number)]);
      for (const log of state.logs) {
        if (Number(log.blockNumber) > state.latest) continue;
        const parsed = poolInterface.parseLog(log);
        if (parsed.name !== 'Transact') continue;
        forest.append(Number(parsed.args.treeNumber), Number(parsed.args.startPosition), [...parsed.args.hash]);
        if (Number(parsed.args.treeNumber) === p.tree_number) roots.add(forest.root(p.tree_number));
      }
      return result(p, { tree_number: p.tree_number, root: p.root, known: roots.has(p.root), block: block(state.latest) });
    }
    if (p.kind === 'nullifiers') {
      const spent = new Set();
      for (const log of state.logs) {
        if (Number(log.blockNumber) > state.latest) continue;
        const parsed = poolInterface.parseLog(log);
        if (parsed.name === 'Nullified' && Number(parsed.args.treeNumber) === p.tree_number) {
          for (const n of parsed.args.nullifier) spent.add(n);
        }
      }
      return result(p, { tree_number: p.tree_number, nullifiers: p.nullifiers,
        spent: p.nullifiers.map(n => spent.has(n)), block: block(state.latest) });
    }
    throw Error(`Unexpected RPC ${p.kind}`);
  };
  const engine = new LocalPrivateEngine(manifest, port, { clock: () => 1000 });
  await engine.initialize({
    wallet_identity: h(3), spending_private_key_hex: '11'.repeat(32), viewing_private_key_hex: '12'.repeat(32),
    encrypted_state_key_hex: '13'.repeat(32), domain, fee_recipient_address: payee.address,
    fee_recipient_commitment: fee.fee_recipient_commitment, creation_block: 0,
  });
  return { engine, state, calls };
}

function copyForest(source) {
  const target = new MerkleForest();
  for (const [tree, value] of source.trees) target.trees.set(tree, {
    count: value.count, levels: value.levels.map(level => new Map(level)),
  });
  return target;
}

function scannedPrefix(engine, state, { block = 7, count = 65534 } = {}) {
  const forest = denseTree(0, [], count);
  state.prefix = { block, forest };
  engine.forest.append(0, 0, [hex32(999)]);
  engine.forest.active = copyForest(forest);
  engine.cursor = block; engine.blockHash = h(block); engine.complete = false;
}

const scanParameters = (cutoff = 10, final = false) => ({
  recovery_id: h(100), wallet_binding: h(101), cutoff_block_number: cutoff,
  cutoff_block_hash: h(cutoff), final_scan: final,
});
function planFor(scan, { recipient = own.address, feeBudget = '100' } = {}) {
  return {
    schema_version: 1, recovery_id: h(100), own_private_recipient: recipient,
    initial_scan: { ...scan, scan_digest: h(103) }, max_total_fee_units: feeBudget,
    approved_at: 1000, plan_digest: h(104),
  };
}
const refs = plan => plan.initial_scan.spendable_notes.map(entry => ({
  tree_number: entry.tree_number, commitment: entry.note.commitment, nullifier: entry.note.nullifier,
}));
const prepareParameters = plan => ({
  plan_json: JSON.stringify(plan), intent_id: h(105), fee_schedule: fee,
  remaining_note_commitments: refs(plan),
});

test('durable cutoff is latest, and no empty recovery scan clears an unfinalized interval', async () => {
  const { engine, state, calls } = await fixture([], { latest: 10, finalized: 5 });
  const cutoff = JSON.parse((await captureRecoveryCutoff(engine, { recovery_id: h(100) })).cutoff_json);
  assert.equal(cutoff.block_number, 10); assert.equal(cutoff.block_hash, h(10));
  await assert.rejects(scanRecovery(engine, scanParameters()), /RECOVERY_WAITING_FOR_FINALITY/);
  assert.equal(calls.some(call => call.kind === 'logs'), false);
  state.finalized = 10;
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json);
  assert.equal(scan.complete_from_genesis, true); assert.equal(scan.scanned_through_block, 10);
  assert.deepEqual(scan.spendable_notes, []); assert.equal(scan.scan_digest, '');
  engine.close();
});

test('full SDK scan retains pending notes and repeated funded commitments at distinct positions', async () => {
  const same = makeTransfer(own.address, own, 1234567n), foreign = makeTransfer(payee.address, own, 20n);
  const { engine } = await fixture([
    await encryptedEvent(0, 0, [same, foreign], 3), await encryptedEvent(0, 2, [same], 4),
  ]);
  const pending = `0:${hex32(TransactNote.getNullifier(engine.own.nullifyingKey, 0))}`;
  engine.pending.add(pending);
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json);
  assert.equal(scan.spendable_notes.length, 2);
  assert.equal(scan.spendable_notes[0].note.commitment, scan.spendable_notes[1].note.commitment);
  assert.notEqual(scan.spendable_notes[0].note.nullifier, scan.spendable_notes[1].note.nullifier);
  assert.deepEqual(scan.spendable_notes.map(n => n.note.units), ['1234567', '1234567']);
  assert.equal(engine.available().length, 1); assert.equal(engine.pending.has(pending), true);
  engine.close();
});

test('historical initial cutoff is scanned exactly after finalized head has advanced', async () => {
  const oldNote = makeTransfer(own.address, own, 100n), laterNote = makeTransfer(own.address, own, 200n);
  const { engine, calls } = await fixture([
    await encryptedEvent(0, 0, [oldNote], 3), await encryptedEvent(0, 1, [laterNote], 12),
  ], { latest: 20, finalized: 20 });
  const scan = JSON.parse((await scanRecovery(engine, scanParameters(10))).scan_json);
  assert.equal(scan.scanned_through_block, 10); assert.equal(scan.finalized_height, 20);
  assert.deepEqual(scan.spendable_notes.map(n => n.note.units), ['100']);
  assert(calls.filter(call => call.kind === 'logs').every(call => call.to_block <= 10));
  engine.close();
});

test('a historically known prefix root cannot certify truncated current-tail logs as complete', async () => {
  const a = makeTransfer(own.address, own, 30n), b = makeTransfer(own.address, own, 40n);
  const earlier = await encryptedEvent(0, 0, [a], 3), omitted = await encryptedEvent(0, 1, [b], 9);
  const { engine, state } = await fixture([earlier, omitted]);
  state.omittedTransactions.add(omitted.transactionHash);
  await assert.rejects(scanRecovery(engine, scanParameters()), /SYNC_INCOMPLETE/);
  assert.equal(engine.complete, false);
  engine.close();
});

test('a rejected persisted omission is rescanned on retry without dropping pending reservations', async () => {
  const deposit = await encryptedEvent(0, 0, [makeTransfer(own.address, own, 100n)], 3);
  const { engine, state, calls } = await fixture([deposit]);
  const reserved = `0:${h(123)}`;
  engine.pending.add(reserved);
  state.omittedTransactions.add(deposit.transactionHash);
  await assert.rejects(scanRecovery(engine, scanParameters()), /SYNC_INCOMPLETE/);
  assert.equal(engine.complete, false); assert.equal(engine.pending.has(reserved), true);
  state.omittedTransactions.clear(); calls.length = 0;
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json);
  assert.equal(scan.spendable_notes.length, 1);
  assert.equal(scan.spendable_notes[0].note.units, '100');
  assert(calls.some(p => p.kind === 'logs' && p.from_block <= 3 && p.to_block >= 3));
  assert.equal(engine.pending.has(reserved), true);
  engine.close();
});

test('real self-sweep preparation can consume an old pending proof without freeing ordinary reservations', async () => {
  const input = makeTransfer(own.address, own, 1000001n);
  const { engine } = await fixture([await encryptedEvent(0, 0, [input], 3)]);
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json), plan = planFor(scan);
  const pending = `0:${scan.spendable_notes[0].note.nullifier}`;
  engine.pending.add(pending); assert.equal(engine.available().length, 0);
  const prepared = await prepareRecoveryBatch(engine, prepareParameters(plan));
  const intent = JSON.parse(prepared.intent_json), review = engine.reviews.get(prepared.review_id);
  assert.equal(intent.recipient, engine.own.address); assert.equal(intent.amount_units, '999994');
  assert.equal(intent.change_units, '0'); assert.equal(intent.change_output_commitment, null);
  assert.deepEqual(intent.input_notes, scan.spendable_notes.map(n => n.note));
  assert.equal(prepared.includes_all_spendable_notes, true);
  assert.equal(review.request.privateInputs.valueIn[0], 1000001n);
  assert.deepEqual(review.request.publicInputs.nullifiers.map(hex32), intent.input_notes.map(n => n.nullifier));
  assert.equal(engine.pending.has(pending), true); assert.equal(engine.available().length, 0);
  await assert.rejects(prepareRecoveryBatch(engine, prepareParameters(plan)), /RECOVERY_REVIEW_IN_PROGRESS/);
  await engine.cancel({ review_id: prepared.review_id });
  assert.equal(engine.pending.has(pending), true); assert.equal(engine.available().length, 0);
  engine.close();
});

test('batch selection is tree scoped, explicit, and never claims partial selection is wallet-wide Max', async () => {
  const a = makeTransfer(own.address, own, 30n), b = makeTransfer(own.address, own, 40n);
  const { engine } = await fixture([await encryptedEvent(0, 0, [a], 3)]);
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json);
  // Selection is tested against a legitimate pre-scanned full tree. The
  // public filler subtree uses real pinned Poseidon hashes and a count of
  // 65,536; no impossible early rollover or lowered capacity is accepted.
  // Separate scan tests exercise canonical log completeness and reject gaps.
  engine.forest.active = denseTree(0, [hex32(a.hash)]);
  await engine.applyLog(await encryptedEvent(1, 0, [b], 4), new Set());
  scan.state_root = engine.forest.root(1);
  scan.spendable_notes = [...engine.notes.values()].map(entry => ({
    tree_number: entry.tree, created_block_number: entry.block,
    note: { commitment: hex32(entry.note.hash), nullifier: entry.nullifier, units: entry.note.value.toString() },
  }));
  const plan = planFor(scan);
  assert.equal(scan.spendable_notes[0].note.nullifier, scan.spendable_notes[1].note.nullifier);
  const first = await prepareRecoveryBatch(engine, prepareParameters(plan));
  const firstIntent = JSON.parse(first.intent_json);
  assert.equal(firstIntent.tree_number, 0); assert.equal(firstIntent.amount_units, '23');
  assert.equal(first.includes_all_spendable_notes, false);
  const second = await prepareRecoveryBatch(engine, {
    ...prepareParameters(plan), intent_id: h(106), remaining_note_commitments: refs(plan).slice(1),
  });
  assert.equal(JSON.parse(second.intent_json).tree_number, 1);
  assert.equal(second.recipient_units, '33'); assert.equal(second.includes_all_spendable_notes, false);
  engine.close();
});

test('recovery scan rejects an apparently canonical but impossible early tree rollover', async () => {
  const a = makeTransfer(own.address, own, 30n), b = makeTransfer(own.address, own, 40n);
  const { engine } = await fixture([
    await encryptedEvent(0, 0, [a], 3), await encryptedEvent(1, 0, [b], 4),
  ]);
  await assert.rejects(scanRecovery(engine, scanParameters()), /INCOMPLETE_TREE/);
  assert.equal(engine.complete, false);
  assert.equal(engine.forest.count(0), 1);
  assert.equal(engine.forest.activeTree, 0);
  engine.close();
});

test('a missing old-tree tail before rollover fails the previous-block closure checkpoint', async () => {
  const tail = await encryptedEvent(0, 65534, [makeTransfer(own.address, own, 5n)], 8);
  const rollover = await encryptedEvent(1, 0,
    [makeTransfer(own.address, own, 6n), makeTransfer(own.address, own, 7n), makeTransfer(own.address, own, 8n)], 9);
  const { engine, state, calls } = await fixture([tail, rollover]);
  scannedPrefix(engine, state);
  state.omittedTransactions.add(tail.transactionHash);
  assert(65534 + 3 > 65536, 'the old inequality alone cannot detect this missing one-leaf event');
  await assert.rejects(scanRecovery(engine, scanParameters()), /SYNC_INCOMPLETE/);
  assert.equal(engine.complete, false);
  assert(calls.some(p => p.kind === 'checkpoint' && p.number === 8));
  assert(!calls.some(p => p.kind === 'block_logs'), 'the prefix mismatch is detected first');
  engine.close();
});

test('a missing same-block old-tree tail fails complete canonical receipt coverage', async () => {
  const tail = await encryptedEvent(0, 65534, [makeTransfer(own.address, own, 5n)], 9, 0);
  const rollover = await encryptedEvent(1, 0,
    [makeTransfer(own.address, own, 6n), makeTransfer(own.address, own, 7n), makeTransfer(own.address, own, 8n)], 9, 1);
  tail.transactionHash = h(2009); // Distinct canonical transactions in the same block.
  rollover.transactionIndex = '0x1';
  const { engine, state, calls } = await fixture([tail, rollover]);
  scannedPrefix(engine, state);
  state.omittedTransactions.add(tail.transactionHash);
  await assert.rejects(scanRecovery(engine, scanParameters()), /SYNC_INCOMPLETE/);
  assert.equal(engine.complete, false);
  assert(calls.some(p => p.kind === 'checkpoint' && p.number === 8));
  assert(calls.some(p => p.kind === 'block_logs' && p.number === 9));
  assert.equal(engine.forest.activeTree, 0, 'the incomplete rollover block is rejected before mutation');
  engine.close();
});

test('complete same-block tail and rollover pass both closure checkpoints with exact owned notes', async () => {
  const tail = await encryptedEvent(0, 65534, [makeTransfer(own.address, own, 5n)], 9, 0);
  const outputs = [makeTransfer(own.address, own, 6n), makeTransfer(own.address, own, 7n), makeTransfer(own.address, own, 8n)];
  const rollover = await encryptedEvent(1, 0, outputs, 9, 1);
  tail.transactionHash = h(2009); rollover.transactionIndex = '0x1';
  const { engine, state, calls } = await fixture([tail, rollover]);
  scannedPrefix(engine, state);
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json);
  assert.equal(engine.complete, true);
  assert.equal(engine.forest.count(0), 65535); assert.equal(engine.forest.count(1), 3);
  assert.equal(scan.spendable_notes.length, 4);
  assert.deepEqual(scan.spendable_notes.slice(1).map(entry => entry.note.commitment), outputs.map(note => hex32(note.hash)));
  assert(calls.some(p => p.kind === 'checkpoint' && p.number === 8));
  assert(calls.some(p => p.kind === 'block_logs' && p.number === 9));
  assert(calls.some(p => p.kind === 'checkpoint' && p.number === 9));
  engine.close();
});

test('recovery refuses external recipient, conflicting note opening, duplicate or vanished input, and incomplete scan', async () => {
  const input = makeTransfer(own.address, own, 100n);
  const { engine } = await fixture([await encryptedEvent(0, 0, [input], 3)]);
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json), plan = planFor(scan);
  await assert.rejects(prepareRecoveryBatch(engine, prepareParameters(planFor(scan, { recipient: payee.address }))), /RECOVERY_SELF_TRANSFER_ONLY/);
  const changed = structuredClone(plan); changed.initial_scan.spendable_notes[0].note.units = '101';
  await assert.rejects(prepareRecoveryBatch(engine, prepareParameters(changed)), /RECOVERY_NOTE_MISMATCH/);
  await assert.rejects(prepareRecoveryBatch(engine, { ...prepareParameters(plan), remaining_note_commitments: [...refs(plan), ...refs(plan)] }), /RECOVERY_NOTE_MISMATCH/);
  const unknown = refs(plan); unknown[0].nullifier = h(900);
  await assert.rejects(prepareRecoveryBatch(engine, { ...prepareParameters(plan), remaining_note_commitments: unknown }), /RECOVERY_NOTE_MISMATCH/);
  const note = [...engine.notes.values()][0]; note.spent = true;
  await assert.rejects(prepareRecoveryBatch(engine, prepareParameters(plan)), /RECOVERY_NEEDS_RECONCILIATION/);
  note.spent = false; engine.complete = false;
  await assert.rejects(prepareRecoveryBatch(engine, prepareParameters(plan)), /SYNC_INCOMPLETE/);
  assert.equal(engine.reviews.size, 0); engine.close();
});

test('canonical cutoff reorg fails and complete rescans recover exact fresh self outputs after nullification', async () => {
  const input = makeTransfer(own.address, own, 100n);
  const { engine, state } = await fixture([await encryptedEvent(0, 0, [input], 3)]);
  const initial = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json);
  state.hashes.set(10, h(500));
  await assert.rejects(scanRecovery(engine, scanParameters()), /RECOVERY_CUTOFF_REORG/);
  await assert.rejects(prepareRecoveryBatch(engine, prepareParameters(planFor(initial))), /RECOVERY_CUTOFF_REORG/);
  state.hashes.delete(10);
  const fresh = makeTransfer(own.address, own, 93n);
  state.logs.push(nullifierEvent(0, [initial.spendable_notes[0].note.nullifier], 11));
  state.logs.push(await encryptedEvent(0, 1, [fresh], 12));
  state.latest = 20; state.finalized = 20;
  const final = JSON.parse((await scanRecovery(engine, scanParameters(10, true))).scan_json);
  assert.equal(final.scanned_through_block, 20); assert.equal(final.cutoff_block_number, 10);
  assert.equal(final.spendable_notes.length, 1);
  assert.equal(final.spendable_notes[0].note.commitment, hex32(fresh.hash));
  assert.equal(final.spendable_notes[0].note.units, '93');
  assert.equal(final.spendable_notes[0].created_block_number, 12);
  engine.close();
});

test('dust cannot be silently omitted and native sealed JSON cannot contain duplicate plan keys', async () => {
  const input = makeTransfer(own.address, own, 7n);
  const { engine } = await fixture([await encryptedEvent(0, 0, [input], 3)]);
  const scan = JSON.parse((await scanRecovery(engine, scanParameters())).scan_json), plan = planFor(scan);
  await assert.rejects(prepareRecoveryBatch(engine, prepareParameters(plan)), /RECOVERY_FEE_EXCEEDS_BALANCE/);
  const canonical = JSON.stringify(plan);
  const duplicate = canonical.replace('"schema_version":1', '"schema_version":1,"schema_version":1');
  await assert.rejects(prepareRecoveryBatch(engine, { ...prepareParameters(plan), plan_json: duplicate }), /INVALID_INPUT/);
  assert.equal(engine.reviews.size, 0); assert.equal(engine.notes.size, 1);
  engine.close();
});
