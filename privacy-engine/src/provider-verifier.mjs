/** Provider reads public proof/ciphertext and decrypts only its own fee note. */
import { candidateInterface, inspectCandidateCall, matchCandidateEnvelope, CANDIDATE } from './candidate-calldata.mjs';
import { verifyCandidateProof } from './local-verifier.mjs';
import { addressData, feeRecipientCommitment, decryptTransact, hex32, parseUnits, TOKEN } from './protocol.mjs';
import { getBytes } from 'ethers';
import keys from '../node_modules/@railgun-community/engine/dist/utils/keys-utils.js';

/**
 * Fee-note evidence ONLY. This function does not verify a ZK proof, authorize
 * admission, establish chain state, or decrypt the principal/change outputs.
 * The inspected object must originate from the local restricted ABI inspector.
 * A caller must separately enforce the shared fee schedule and deployment pins.
 */
export async function decodeProviderFee(inspected, envelope, config) {
  let viewing;
  try {
    matchCandidateEnvelope(inspected, envelope);
    const recipient = config.fee_recipient_address;
    const recipientCommitment = feeRecipientCommitment(recipient);
    if (recipientCommitment !== envelope.fee.fee_recipient_commitment
        || typeof config.fee_viewing_private_key_hex !== 'string'
        || !/^[0-9a-f]{64}$/.test(config.fee_viewing_private_key_hex)
        || /^0+$/.test(config.fee_viewing_private_key_hex)) throw Error('VERIFICATION_FAILED');
    const expectedFee = parseUnits(envelope.fee.fee_units);
    if (expectedFee === 0n) throw Error('VERIFICATION_FAILED');
    viewing = getBytes(`0x${config.fee_viewing_private_key_hex}`);
    const data = addressData(recipient), publicKey = await keys.getPublicViewingKey(viewing);
    if (publicKey.length !== data.viewingPublicKey.length
        || publicKey.some((byte, i) => byte !== data.viewingPublicKey[i])) throw Error('VERIFICATION_FAILED');
    const tx = candidateInterface.decodeFunctionData('transact', inspected.callData)[0][0];
    const index = inspected.outputCommitments.indexOf(envelope.fee.fee_output_commitment);
    if (index < 0) throw Error('VERIFICATION_FAILED');
    const note = await decryptTransact(tx.boundParams.commitmentCiphertext[index],
      inspected.outputCommitments[index], { viewing, data, address: recipient });
    // The decryption helper validates exact commitment, USDC token identity,
    // uint120 value and receiving keys. A hidden sender and empty memo are
    // required so the fee record contains no sender disclosure.
    if (!note || note.value !== expectedFee || note.getSenderAddress() !== undefined || note.memoText) {
      throw Error('VERIFICATION_FAILED');
    }
    return Object.freeze({
      fee_output_commitment: hex32(note.hash), fee_units: note.value.toString(),
      token_contract: TOKEN, fee_recipient_commitment: recipientCommitment,
    });
  } catch {
    throw Error('VERIFICATION_FAILED');
  } finally {
    viewing?.fill(0);
  }
}

/** Always verify the genuine pinned Groth16 proof before using fee evidence. */
export async function verifyProviderEnvelope(envelope, config, artifactReader) {
  const inspected = inspectCandidateCall({ to: CANDIDATE.pool, value: 0n, data: envelope.call_data });
  matchCandidateEnvelope(inspected, envelope);
  const shape = `${String(inspected.nullifiers.length).padStart(2, '0')}x${String(inspected.outputCommitments.length).padStart(2, '0')}`;
  const artifact = config.circuits?.[shape];
  if (!artifact || envelope.domain.verifier_digest !== config.verifier_digest
      || config.additional_native_fee_cap_wei !== '0') throw Error('VERIFICATION_FAILED');
  if (!await verifyCandidateProof(inspected, await artifactReader(artifact.vkey.path), artifact.vkey.sha256)) {
    throw Error('VERIFICATION_FAILED');
  }
  const fee = await decodeProviderFee(inspected, envelope, config);
  return {
    schema_version: 1, operation_digest: envelope.operation_digest, tree_number: inspected.treeNumber,
    call_data: inspected.callData, fee_output_commitment: fee.fee_output_commitment,
    additional_native_fee_cap_wei: '0',
  };
}
