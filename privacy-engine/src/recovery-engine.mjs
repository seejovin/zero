/** Local recovery orchestration over the actual pinned note SDK.
 *
 * No function broadcasts a payment. The native encrypted journal is the
 * authority for cutoff, plan approval, reservations, receipts and completion.
 * RPC responses are normalized canonical-chain adapter observations; they are
 * not advertised as cryptographic consensus proofs by this module.
 */
import {
  TOKEN, CHAIN, TransactNote, hex32, parseUnits, isSupportedNote,
} from './protocol.mjs';

const HASH = /^0x[0-9a-f]{64}$/;
const ZERO = `0x${'0'.repeat(64)}`;
const MAX_NOTES = 1024;
const MAX_JSON = 524288;
const MAX_INPUTS = 11;
const fail = code => { throw new Error(code); };
const hash = value => {
  if (typeof value !== 'string' || !HASH.test(value) || value === ZERO) fail('INVALID_INPUT');
  return value;
};
const integer = (value, maximum = Number.MAX_SAFE_INTEGER) => {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) fail('INVALID_INPUT');
  return value;
};
const fields = (value, expected) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).sort().join(',') !== [...expected].sort().join(',')) fail('INVALID_INPUT');
};
const request = (kind, extra = {}) => ({ schema_version: 1, chain_id: 42161, kind, ...extra });
const keyOf = value => `${value.tree_number}:${value.nullifier}`;
const block = value => {
  if (!value || integer(value.number) === 0) fail('PROTOCOL_MISMATCH');
  hash(value.hash);
  return value;
};
const aggregateUnits = value => {
  if (typeof value !== 'string' || !/^(0|[1-9][0-9]{0,77})$/.test(value)
      || BigInt(value) >= (1n << 256n)) fail('INVALID_INPUT');
  return BigInt(value);
};

function checkedNote(engine, entry) {
  integer(entry.tree, 65535);
  integer(entry.position, 65535);
  integer(entry.block);
  hash(entry.nullifier);
  if (!isSupportedNote(entry.note) || entry.note.value <= 0n || entry.block === 0
      || entry.block > engine.cursor || engine.forest.leaf(entry.tree, entry.position) !== hex32(entry.note.hash)
      || hex32(TransactNote.getNullifier(engine.own.nullifyingKey, entry.position)) !== entry.nullifier
      || entry.note.receiverAddressData.masterPublicKey !== engine.own.data.masterPublicKey
      || entry.note.receiverAddressData.viewingPublicKey.length !== engine.own.viewingPublic.length
      || entry.note.receiverAddressData.viewingPublicKey.some((v, i) => v !== engine.own.viewingPublic[i])) {
    fail('RECOVERY_NOTE_MISMATCH');
  }
  return entry;
}

/** Includes pending notes deliberately: an earlier released proof is precisely
 * why recovery exists. Pending cache entries are not erased or made generally
 * available. Only native-authorized self-sweep preparation may select them. */
function ownedUnspent(engine) {
  const notes = [...engine.notes.values()].filter(note => !note.spent)
    .map(note => checkedNote(engine, note))
    .sort((a, b) => a.tree - b.tree || a.position - b.position);
  if (notes.length > MAX_NOTES) fail('RECOVERY_CAPACITY_EXCEEDED');
  const ids = new Set();
  for (const note of notes) {
    const id = `${note.tree}:${note.nullifier}`;
    if (ids.has(id)) fail('RECOVERY_NOTE_MISMATCH');
    ids.add(id);
  }
  return notes;
}

/** Returns the latest observed head, not merely the latest finalized balance.
 * Android must durably record cutoff_json before calling scanRecovery. */
export async function captureRecoveryCutoff(engine, parameters) {
  engine.requireReady();
  fields(parameters, ['recovery_id']);
  hash(parameters.recovery_id);
  const latest = block((await engine.rpc(request('tip'))).block);
  const canonical = block((await engine.rpc(request('block', { number: latest.number }))).block);
  if (canonical.number !== latest.number || canonical.hash !== latest.hash) fail('SYNC_INCOMPLETE');
  const observed = integer(engine.clock());
  if (!observed) fail('INVALID_INPUT');
  return { cutoff_json: JSON.stringify({
    schema_version: 1, recovery_id: parameters.recovery_id, domain: engine.manifest.domain,
    block_number: latest.number, block_hash: latest.hash, observed_at: observed,
  }) };
}

/** Initial scans stop at the durable cutoff. Final rescans cover the current
 * finalized head. Both retain the original cutoff identity and require a full
 * pool history scan through it; a cached balance or unfinalized zero is rejected.
 * Native privatePrepareRecoveryScan seals the returned canonical local fields. */
export async function scanRecovery(engine, parameters, progress = () => {}) {
  engine.requireReady();
  fields(parameters, ['recovery_id', 'wallet_binding', 'cutoff_block_number', 'cutoff_block_hash', 'final_scan']);
  hash(parameters.recovery_id); hash(parameters.wallet_binding); hash(parameters.cutoff_block_hash);
  if (!integer(parameters.cutoff_block_number) || typeof parameters.final_scan !== 'boolean') fail('INVALID_INPUT');
  const before = block((await engine.rpc(request('head'))).block);
  if (before.number < parameters.cutoff_block_number) fail('RECOVERY_WAITING_FOR_FINALITY');
  const cutoff = block((await engine.rpc(request('block', { number: parameters.cutoff_block_number }))).block);
  if (cutoff.number !== parameters.cutoff_block_number || cutoff.hash !== parameters.cutoff_block_hash) fail('RECOVERY_CUTOFF_REORG');
  const target = parameters.final_scan ? null : parameters.cutoff_block_number;
  await engine.sync({ chain_id: 42161 }, progress, target);
  if (!engine.complete || !Number.isSafeInteger(engine.cursor)
      || engine.cursor < parameters.cutoff_block_number
      || (!parameters.final_scan && engine.cursor !== parameters.cutoff_block_number)) fail('SYNC_INCOMPLETE');
  const after = block((await engine.rpc(request('head'))).block);
  const cutoffAfter = block((await engine.rpc(request('block', { number: parameters.cutoff_block_number }))).block);
  const scanned = block((await engine.rpc(request('block', { number: engine.cursor }))).block);
  if (after.number < before.number || after.number < engine.cursor
      || cutoffAfter.number !== cutoff.number || cutoffAfter.hash !== cutoff.hash
      || scanned.number !== engine.cursor || scanned.hash !== engine.blockHash) fail('SYNC_INCOMPLETE');
  const notes = ownedUnspent(engine);
  const treeNumbers = [...engine.forest.trees.keys()];
  const latestTree = treeNumbers.length ? Math.max(...treeNumbers) : 0;
  const root = engine.forest.root(latestTree);
  const check = await engine.rpc(request('root', { tree_number: latestTree, root }));
  if (check.tree_number !== latestTree || check.root !== root || check.known !== true
      || block(check.block).number < engine.cursor) fail('SYNC_INCOMPLETE');
  // state_root identifies the latest pool tree at this snapshot; individual
  // input tree identities remain explicit on every note and later intent.
  const scan = {
    schema_version: 1, domain: engine.manifest.domain, wallet_binding: parameters.wallet_binding,
    cutoff_block_number: cutoff.number, cutoff_block_hash: cutoff.hash,
    scanned_through_block: engine.cursor, scanned_block_hash: scanned.hash,
    canonical_scanned_block_hash: scanned.hash, finalized_height: after.number,
    state_root: root, complete_from_genesis: true,
    spendable_notes: notes.map(note => ({
      tree_number: note.tree,
      note: { commitment: hex32(note.note.hash), nullifier: note.nullifier, units: note.note.value.toString() },
      created_block_number: note.block,
    })),
    scan_digest: '',
  };
  const text = JSON.stringify(scan);
  if (text.length > MAX_JSON) fail('RECOVERY_CAPACITY_EXCEEDED');
  return { scan_json: text };
}

function localPlan(engine, text) {
  if (typeof text !== 'string' || text.length > MAX_JSON) fail('INVALID_INPUT');
  let plan;
  try { plan = JSON.parse(text); } catch { fail('INVALID_INPUT'); }
  // Native sealed JSON is canonical. Reject duplicate keys and surprising
  // lexical rewrites instead of accepting JSON.parse's last-key-wins behavior.
  if (JSON.stringify(plan) !== text) fail('INVALID_INPUT');
  fields(plan, ['schema_version', 'recovery_id', 'own_private_recipient', 'initial_scan', 'max_total_fee_units', 'approved_at', 'plan_digest']);
  if (plan.schema_version !== 1 || plan.own_private_recipient !== engine.own.address) fail('RECOVERY_SELF_TRANSFER_ONLY');
  hash(plan.recovery_id); hash(plan.plan_digest); integer(plan.approved_at);
  aggregateUnits(plan.max_total_fee_units);
  const scan = plan.initial_scan;
  fields(scan, ['schema_version', 'domain', 'wallet_binding', 'cutoff_block_number', 'cutoff_block_hash',
    'scanned_through_block', 'scanned_block_hash', 'canonical_scanned_block_hash', 'finalized_height',
    'state_root', 'complete_from_genesis', 'spendable_notes', 'scan_digest']);
  engine.checkDomain(scan.domain);
  for (const field of ['wallet_binding', 'cutoff_block_hash', 'scanned_block_hash', 'canonical_scanned_block_hash', 'state_root', 'scan_digest']) hash(scan[field]);
  if (scan.schema_version !== 1 || scan.complete_from_genesis !== true
      || !integer(scan.cutoff_block_number) || scan.scanned_through_block !== scan.cutoff_block_number
      || scan.scanned_block_hash !== scan.cutoff_block_hash || scan.canonical_scanned_block_hash !== scan.cutoff_block_hash
      || integer(scan.finalized_height) < scan.cutoff_block_number
      || !Array.isArray(scan.spendable_notes) || scan.spendable_notes.length > MAX_NOTES) fail('INVALID_INPUT');
  const ids = new Set();
  for (const entry of scan.spendable_notes) {
    fields(entry, ['tree_number', 'note', 'created_block_number']);
    integer(entry.tree_number, 65535);
    fields(entry.note, ['commitment', 'nullifier', 'units']);
    hash(entry.note.commitment); hash(entry.note.nullifier);
    if (parseUnits(entry.note.units) === 0n || !integer(entry.created_block_number)
        || entry.created_block_number > scan.cutoff_block_number) fail('INVALID_INPUT');
    const id = `${entry.tree_number}:${entry.note.nullifier}`;
    if (ids.has(id)) fail('INVALID_INPUT');
    ids.add(id);
  }
  return plan;
}

/** Builds ONE real SDK self-transfer review. The native coordinator supplies
 * remaining inputs after excluding its current recovery batch reservations.
 * Recovery is explicitly multi-batch; this function never advertises a partial
 * selection as a completed wallet-wide Max or releases quarantine itself. */
export async function prepareRecoveryBatch(engine, parameters) {
  engine.requireReady();
  fields(parameters, ['plan_json', 'intent_id', 'fee_schedule', 'remaining_note_commitments']);
  hash(parameters.intent_id);
  if (!engine.complete) fail('SYNC_INCOMPLETE');
  const plan = localPlan(engine, parameters.plan_json);
  if (engine.cursor < plan.initial_scan.cutoff_block_number) fail('SYNC_INCOMPLETE');
  const canonical = block((await engine.rpc(request('block', { number: plan.initial_scan.cutoff_block_number }))).block);
  if (canonical.number !== plan.initial_scan.cutoff_block_number || canonical.hash !== plan.initial_scan.cutoff_block_hash) fail('RECOVERY_CUTOFF_REORG');
  const requested = parameters.remaining_note_commitments;
  if (!Array.isArray(requested) || !requested.length || requested.length > MAX_NOTES) fail('INVALID_INPUT');
  const unspent = ownedUnspent(engine), byId = new Map(unspent.map(note => [`${note.tree}:${note.nullifier}`, note]));
  const allowed = new Map(plan.initial_scan.spendable_notes.map(entry => [`${entry.tree_number}:${entry.note.nullifier}`, entry]));
  const seen = new Set(), remaining = [];
  for (const ref of requested) {
    fields(ref, ['tree_number', 'commitment', 'nullifier']);
    integer(ref.tree_number, 65535); hash(ref.commitment); hash(ref.nullifier);
    const id = keyOf(ref), approved = allowed.get(id), note = byId.get(id);
    if (seen.has(id) || !approved || approved.note.commitment !== ref.commitment) fail('RECOVERY_NOTE_MISMATCH');
    if (!note) fail('RECOVERY_NEEDS_RECONCILIATION');
    if (hex32(note.note.hash) !== ref.commitment || note.note.value.toString() !== approved.note.units
        || note.block !== approved.created_block_number) fail('RECOVERY_NOTE_MISMATCH');
    seen.add(id); remaining.push(note);
  }
  remaining.sort((a, b) => a.tree - b.tree || a.position - b.position);
  const groups = new Map();
  for (const note of remaining) {
    if (!groups.has(note.tree)) groups.set(note.tree, []);
    groups.get(note.tree).push(note);
  }
  if (!parameters.fee_schedule || typeof parameters.fee_schedule !== 'object') fail('INVALID_INPUT');
  const fee = parseUnits(parameters.fee_schedule.fee_units);
  if (!fee || fee > aggregateUnits(plan.max_total_fee_units)) fail('RECOVERY_FEE_EXCEEDS_BALANCE');
  let selected = null;
  for (const group of groups.values()) {
    for (let size = Math.min(group.length, MAX_INPUTS); size > 0; size--) {
      if (!engine.manifest.circuits[`${String(size).padStart(2, '0')}x03`]) continue;
      const choice = group.slice(0, size), total = choice.reduce((sum, note) => sum + note.note.value, 0n);
      if (total > fee && total - fee < (1n << 120n)) { selected = choice; break; }
    }
    if (selected) break;
  }
  if (!selected) fail('RECOVERY_FEE_EXCEEDS_BALANCE');
  const selectedIds = new Set(selected.map(note => `${note.tree}:${note.nullifier}`));
  for (const review of engine.reviews.values()) {
    if (review.selected.some(note => selectedIds.has(`${note.tree}:${note.nullifier}`))) fail('RECOVERY_REVIEW_IN_PROGRESS');
  }
  if (engine.reviews.size >= 4) fail('UNAVAILABLE');
  // Narrow projection: the ordinary engine's available()/pending state is not
  // modified. prepare() creates genuine randomized notes and SDK transaction
  // requests, but it writes only this view's private review map until validated.
  const view = Object.create(engine);
  view.reviews = new Map();
  view.available = () => [...selected];
  let prepared;
  try {
    prepared = await engine.prepare.call(view, {
      intent_id: parameters.intent_id, chain_id: CHAIN.id, token_contract: TOKEN,
      recipient: engine.own.address, amount_units: null, maximum: true,
      fee_schedule: parameters.fee_schedule,
    });
    const intent = JSON.parse(prepared.intent_json), review = view.reviews.get(prepared.review_id);
    const total = selected.reduce((sum, note) => sum + note.note.value, 0n);
    if (!review || intent.recipient !== engine.own.address || intent.tree_number !== selected[0].tree
        || intent.change_units !== '0' || intent.change_output_commitment !== null
        || BigInt(intent.amount_units) !== total - fee || intent.input_notes.length !== selected.length
        || intent.input_notes.some((note, i) => note.commitment !== hex32(selected[i].note.hash)
          || note.nullifier !== selected[i].nullifier || note.units !== selected[i].note.value.toString())
        || engine.reviews.size >= 4 || engine.reviews.has(prepared.review_id)) fail('RECOVERY_NOTE_MISMATCH');
    engine.reviews.set(prepared.review_id, review);
    view.reviews.delete(prepared.review_id);
    return {
      ...prepared,
      includes_all_spendable_notes: selected.length === unspent.length,
    };
  } finally {
    // On rejection, erase temporary witness material using the same ordinary
    // engine cancellation method. No partial review is installed globally.
    for (const reviewId of [...view.reviews.keys()]) await engine.cancel.call(view, { review_id: reviewId });
  }
}
