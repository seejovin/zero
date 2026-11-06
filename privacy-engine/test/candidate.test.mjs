import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { ZeroAddress, ZeroHash } from 'ethers';
import { candidateInterface, CANDIDATE, inspectCandidateCall, matchCandidateEnvelope,
  requireProductionPrivateEngine } from '../src/candidate-calldata.mjs';
import { pinnedBytes, verifyCandidateProof } from '../src/local-verifier.mjs';

const hash = n => `0x${n.toString(16).padStart(64, '0')}`;
// Deliberately invalid Groth16 points: these are ABI/negative fixtures only.
function fixture() {
  return {
    proof: { a: { x: 1n, y: 2n }, b: { x: [3n, 4n], y: [5n, 6n] }, c: { x: 7n, y: 8n } },
    merkleRoot: hash(20), nullifiers: [hash(21), hash(22)], commitments: [hash(23), hash(24)],
    boundParams: { treeNumber: 1, minGasPrice: 0, unshield: 0, chainID: 42161,
      adaptContract: ZeroAddress, adaptParams: ZeroHash, commitmentCiphertext: [0, 1].map(n => ({
        ciphertext: [hash(30 + n), hash(31), hash(32), hash(33)],
        blindedSenderViewingKey: hash(40), blindedReceiverViewingKey: hash(41), annotationData: '0x0102', memo: '0x',
      })) },
    unshieldPreimage: { npk: ZeroHash, token: { tokenType: 0, tokenAddress: ZeroAddress, tokenSubID: 0 }, value: 0 },
  };
}
function call(tx = fixture()) {
  return { to: CANDIDATE.pool, value: 0n, data: candidateInterface.encodeFunctionData('transact', [[tx]]) };
}
test('real SDK ABI yields exact typed envelope fields without claiming proof validity', () => {
  const i = inspectCandidateCall(call());
  assert.equal(i.proofVerified, false);
  assert.equal(i.publicSignals.length, 6);
  assert.deepEqual(i.snarkProof.pi_b[0], ['4', '3']);
  const e = { schema_version: 2, tree_number: i.treeNumber, domain: { protocol: CANDIDATE.protocol, chain_id: 42161, pool_contract: CANDIDATE.pool,
    token_contract: CANDIDATE.token, token_decimals: 6 }, state_root: i.stateRoot,
    nullifiers: i.nullifiers, output_commitments: i.outputCommitments, encrypted_outputs: i.encryptedOutputs,
    proof: i.proof, call_data: i.callData, fee: { fee_output_commitment: i.outputCommitments[1] } };
  assert.equal(matchCandidateEnvelope(i, e).calldataMatched, true);
  for (const changed of [{ ...e, state_root: hash(99) }, { ...e, call_data: e.call_data + '00' },
    { ...e, fee: { fee_output_commitment: hash(99) } }, { ...e, encrypted_outputs: [] }]) {
    assert.throws(() => matchCandidateEnvelope(i, changed));
  }
});
test('rejects unshield, external calls, wrong network, native value and ambiguous encoding', () => {
  const changes = [
    x => { x.boundParams.unshield = 1; }, x => { x.boundParams.chainID = 1; },
    x => { x.boundParams.adaptContract = '0x1111111111111111111111111111111111111111'; },
    x => { x.boundParams.adaptParams = hash(1); }, x => { x.unshieldPreimage.value = 1; },
    x => { x.unshieldPreimage.token.tokenAddress = CANDIDATE.token; },
    x => { x.nullifiers.push(x.nullifiers[0]); }, x => { x.boundParams.commitmentCiphertext.pop(); },
    x => { x.boundParams.minGasPrice = 1n << 48n; },
  ];
  for (const change of changes) { const tx = fixture(); change(tx); assert.throws(() => inspectCandidateCall(call(tx))); }
  assert.throws(() => inspectCandidateCall({ ...call(), value: 1n }));
  assert.throws(() => inspectCandidateCall({ ...call(), to: ZeroAddress }));
  assert.throws(() => inspectCandidateCall({ ...call(), data: call().data + '00' }));
  assert.throws(() => inspectCandidateCall({ ...call(), data: candidateInterface.encodeFunctionData('transact', [[fixture(), fixture()]]) }));
});
test('requires trusted exact artifact bytes and refuses missing or unrelated verification keys', async () => {
  const b = Buffer.from('{"protocol":"not-groth16"}');
  const pin = createHash('sha256').update(b).digest('hex');
  assert.deepEqual(pinnedBytes(b, pin), new Uint8Array(b));
  assert.throws(() => pinnedBytes(b, '00'.repeat(32)));
  assert.throws(() => pinnedBytes(b, pin, 1));
  await assert.rejects(verifyCandidateProof(inspectCandidateCall(call()), b, pin));
  await assert.rejects(verifyCandidateProof({ publicSignals: [] }, b, pin));
});
test('production gate cannot be enabled by an environment variable', () => {
  process.env.LOCAL_WALLET_PRIVATE_ENABLED = 'true';
  try { assert.throws(requireProductionPrivateEngine); }
  finally { delete process.env.LOCAL_WALLET_PRIVATE_ENABLED; }
});
