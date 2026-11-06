/** Explicit local proof verification; SDK success is never treated as proof validity. */
import { sha256, getBytes } from 'ethers';
import { groth16 } from 'snarkjs';
import { isInspectedCandidate } from './candidate-calldata.mjs';

export function pinnedBytes(bytes, trustedSha256, maximum = 2_000_000) {
  if (!(bytes instanceof Uint8Array) || bytes.length === 0 || bytes.length > maximum
      || !/^[0-9a-f]{64}$/.test(trustedSha256)) throw new Error('Untrusted proof artifact');
  const copy = new Uint8Array(bytes);
  if (sha256(copy).slice(2) !== trustedSha256) {
    throw new Error('Untrusted proof artifact');
  }
  return copy;
}

/**
 * The expected pin must be from an independently reviewed circuit/deployment
 * manifest. Pinning a key supplied with a proof does not authenticate that key.
 * No approved pin is supplied by this candidate and no network is contacted.
 * Cryptographic validity alone does not check fee, canonical state or privacy.
 */
export async function verifyCandidateProof(inspected, vkeyBytes, trustedSha256) {
  if (!isInspectedCandidate(inspected)) throw new Error('Calldata must be inspected locally');
  let key;
  try {
    key = JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(pinnedBytes(vkeyBytes, trustedSha256)));
    if (key.protocol !== 'groth16' || key.curve !== 'bn128'
        || key.nPublic !== inspected.publicSignals.length
        || !Array.isArray(key.IC) || key.IC.length !== key.nPublic + 1) {
      throw new Error('Unsupported verification key');
    }
  } catch { throw new Error('Untrusted proof artifact'); }
  try {
    return await groth16.verify(key, inspected.publicSignals.map(String), inspected.snarkProof) === true;
  } catch {
    return false;
  }
}
