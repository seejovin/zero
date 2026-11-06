/** Actual SDK encryption/opening tests. These ABI fixtures deliberately have
 * invalid SNARK points: a positive fee opening must never imply admission. */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ZeroAddress, ZeroHash, sha256 } from 'ethers';
import {
  localKeys, makeTransfer, Transaction, TransactNote, walletFacade, NO_ADAPT,
  VERSION, CHAIN, TOKEN, hex32, getTokenDataERC20, feeRecipientCommitment,
  OutputType, addressData, decryptTransact,
} from '../src/protocol.mjs';
import { MerkleForest } from '../src/merkle-forest.mjs';
import { candidateInterface, CANDIDATE, inspectCandidateCall } from '../src/candidate-calldata.mjs';
import { decodeProviderFee, verifyProviderEnvelope } from '../src/provider-verifier.mjs';

const h = n => `0x${BigInt(n).toString(16).padStart(64, '0')}`;
const sender = await localKeys('31'.repeat(32), '32'.repeat(32));
const receiver = await localKeys('41'.repeat(32), '42'.repeat(32));
const provider = await localKeys('51'.repeat(32), '52'.repeat(32));
const config = {
  verifier_digest: h(100), additional_native_fee_cap_wei: '0',
  fee_recipient_address: provider.address, fee_viewing_private_key_hex: '52'.repeat(32),
};
const domain = {
  protocol: CANDIDATE.protocol, chain_id: 42161, pool_contract: CANDIDATE.pool,
  token_contract: TOKEN, token_decimals: 6, verifier_digest: config.verifier_digest,
};

function envelopeFor(tx, feeCommitment) {
  const call = candidateInterface.encodeFunctionData('transact', [[tx]]);
  const inspected = inspectCandidateCall({ to: CANDIDATE.pool, value: 0n, data: call });
  return {
    inspected,
    envelope: {
      schema_version: 2, domain, tree_number: inspected.treeNumber,
      state_root: inspected.stateRoot, nullifiers: [...inspected.nullifiers],
      output_commitments: [...inspected.outputCommitments], encrypted_outputs: [...inspected.encryptedOutputs],
      proof: inspected.proof, call_data: inspected.callData, operation_digest: h(101),
      fee: {
        schedule_id: h(102), fee_units: '7', fee_recipient_commitment: feeRecipientCommitment(provider.address),
        fee_output_commitment: feeCommitment, max_native_fee_wei: '1000000000000000',
        gas_limit: 1000000, quote_valid_until: 2000,
      },
    },
  };
}

async function fixture({ discloseSender = false, memo, feeAsset = TOKEN } = {}) {
  const input = makeTransfer(sender.address, sender, 10000n);
  const principal = makeTransfer(receiver.address, sender, 9000n);
  const serviceFee = TransactNote.createTransfer(addressData(provider.address), sender.data, 7n,
    getTokenDataERC20(feeAsset), discloseSender, OutputType.BroadcasterFee, memo);
  const change = makeTransfer(sender.address, sender, 993n, OutputType.Change);
  const forest = new MerkleForest(); forest.append(0, 0, [hex32(input.hash)]);
  const transaction = new Transaction(CHAIN, getTokenDataERC20(TOKEN), 0,
    [{ note: input, position: 0 }], [principal, serviceFee, change], NO_ADAPT);
  const request = await transaction.generateTransactionRequest(walletFacade(sender, forest), VERSION, '', { minGasPrice: 0n });
  const tx = {
    proof: { a: { x: 1n, y: 2n }, b: { x: [3n, 4n], y: [5n, 6n] }, c: { x: 7n, y: 8n } },
    merkleRoot: hex32(request.publicInputs.merkleRoot),
    nullifiers: request.publicInputs.nullifiers.map(hex32),
    commitments: request.publicInputs.commitmentsOut.map(hex32), boundParams: request.boundParams,
    unshieldPreimage: { npk: ZeroHash, token: { tokenType: 0, tokenAddress: ZeroAddress, tokenSubID: 0 }, value: 0 },
  };
  return { tx, ...envelopeFor(tx, hex32(serviceFee.hash)), principal, serviceFee, change };
}

test('provider opens its exact fee using only the fee viewing key, and cannot open beneficiary/change', async () => {
  const { inspected, envelope, tx } = await fixture();
  const evidence = await decodeProviderFee(inspected, envelope, config);
  assert.deepEqual(evidence, {
    fee_output_commitment: envelope.fee.fee_output_commitment, fee_units: '7', token_contract: TOKEN,
    fee_recipient_commitment: envelope.fee.fee_recipient_commitment,
  });
  assert(Object.isFrozen(evidence));
  const viewingOnly = { viewing: provider.viewing, data: provider.data, address: provider.address };
  for (const index of [0, 2]) {
    assert.equal(await decryptTransact(tx.boundParams.commitmentCiphertext[index],
      envelope.output_commitments[index], viewingOnly), null);
  }
  const encoded = JSON.stringify(evidence);
  for (const privateValue of [sender.address, receiver.address, '9000', '993']) assert.equal(encoded.includes(privateValue), false);
  assert.equal(evidence.verified, undefined); assert.equal(evidence.proofVerified, undefined);
});

test('wrong viewing keys, amounts, payee binding and output substitutions are rejected', async () => {
  const { inspected, envelope } = await fixture();
  for (const key of ['42'.repeat(32), '00'.repeat(32), 'not-a-key']) {
    await assert.rejects(decodeProviderFee(inspected, envelope, { ...config, fee_viewing_private_key_hex: key }), /VERIFICATION_FAILED/);
  }
  for (const units of ['8', '0', '07', '1.5', (1n << 120n).toString()]) {
    await assert.rejects(decodeProviderFee(inspected, { ...envelope, fee: { ...envelope.fee, fee_units: units } }, config), /VERIFICATION_FAILED/);
  }
  for (const override of [
    { fee_recipient_commitment: h(999) },
    { fee_output_commitment: envelope.output_commitments[0] },
    { fee_output_commitment: envelope.output_commitments[2] },
  ]) {
    await assert.rejects(decodeProviderFee(inspected, { ...envelope, fee: { ...envelope.fee, ...override } }, config), /VERIFICATION_FAILED/);
  }
  await assert.rejects(decodeProviderFee({ ...inspected }, envelope, config), /VERIFICATION_FAILED/);
});

test('modified ciphertext is rejected even when exact ABI and envelope fields are updated together', async () => {
  const { tx, envelope } = await fixture();
  const changed = structuredClone(tx);
  changed.boundParams.commitmentCiphertext[1].ciphertext[2] = h(777);
  const rebound = envelopeFor(changed, envelope.fee.fee_output_commitment);
  await assert.rejects(decodeProviderFee(rebound.inspected, rebound.envelope, config), /VERIFICATION_FAILED/);
});

test('sender disclosure and a nonempty memo in the fee note are refused', async () => {
  for (const options of [{ discloseSender: true }, { memo: 'unwanted application identifier' }]) {
    const { inspected, envelope } = await fixture(options);
    await assert.rejects(decodeProviderFee(inspected, envelope, config), /VERIFICATION_FAILED/);
  }
});

test('a different asset encrypted for the right provider cannot be counted as the USDC fee', async () => {
  const { inspected, envelope } = await fixture({ feeAsset: '0x1111111111111111111111111111111111111111' });
  await assert.rejects(decodeProviderFee(inspected, envelope, config), /VERIFICATION_FAILED/);
});

test('a decrypted fee does not bypass the mandatory pinned Groth16 verification', async () => {
  const { inspected, envelope } = await fixture();
  await decodeProviderFee(inspected, envelope, config);
  const bytes = new TextEncoder().encode('{"protocol":"invalid"}');
  let reads = 0;
  const cfg = { ...config, circuits: { '01x03': { vkey: { path: 'artifacts/01x03.vkey.json', sha256: sha256(bytes).slice(2) } } } };
  await assert.rejects(verifyProviderEnvelope(envelope, cfg, async path => {
    reads++; assert.equal(path, 'artifacts/01x03.vkey.json'); return bytes;
  }), /Untrusted proof artifact/);
  assert.equal(reads, 1);
});
