/**
 * Canonical inspection of the actual pinned RAILGUN V2 ABI, shared by the
 * offline APK engine and the provider's restricted proof-verifier process.
 * A shape-valid calldata fixture is NOT a valid zero-knowledge proof.
 */
import ABIRailgunSmartWallet from '../node_modules/@railgun-community/engine/dist/abi/V2.1/RailgunSmartWallet.json' with { type: 'json' };
import { AbiCoder, Interface, ZeroAddress, ZeroHash, keccak256 } from 'ethers';

export const CANDIDATE = Object.freeze({
  protocol: 'railgun-v2-candidate', chainId: 42161n,
  pool: '0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9',
  token: '0xaf88d065e77c8cc2239327c5edb3a432268e5831',
  productionEnabled: false,
});
const FIELD = 21888242871839275222246405745257275088548364400416034343698204186575808495617n;
export const candidateInterface = new Interface(ABIRailgunSmartWallet);
const shieldEvent = ABIRailgunSmartWallet.find(x=>x.type==='event'&&x.name==='Shield');
export const poolInterface = new Interface([...ABIRailgunSmartWallet.filter(x=>x.type==='event'), {...shieldEvent,inputs:shieldEvent.inputs.slice(0,4)}]);
const coder = AbiCoder.defaultAbiCoder();
const transact = candidateInterface.getFunction('transact');
const transactionType = transact.inputs[0].arrayChildren;
const proofType = transactionType.components.find(x => x.name === 'proof');
const boundsType = transactionType.components.find(x => x.name === 'boundParams');
const ciphertextType = boundsType.components.find(x => x.name === 'commitmentCiphertext').arrayChildren;
const qualified = new WeakSet();
const fail = () => { throw new Error('Private calldata qualification rejected'); };
const bytes = (value, max) => typeof value === 'string' && /^0x(?:[0-9a-f]{2})*$/.test(value)
  && value.length <= 2 + 2 * max;
const scalar = value => typeof value === 'string' && /^0x[0-9a-f]{64}$/.test(value)
  && BigInt(value) > 0n && BigInt(value) < FIELD;
function uniqueScalars(values) {
  if (values.length < 1 || values.length > 16 || !values.every(scalar)
      || new Set(values).size !== values.length) fail();
}

/**
 * Exact pure-private call inspection, before any provider/simulation.
 * The caller must separately verify Groth16, canonical state, private output
 * decryption, token/fee binding and the circuit/deployment release manifest.
 */
export function inspectCandidateCall({ to, value, data }) {
  if (to !== CANDIDATE.pool || value !== 0n || !bytes(data, 131072) || data.length < 10) fail();
  let tx;
  try {
    const args = candidateInterface.decodeFunctionData(transact, data);
    // Reject trailing data, alternate encodings and multi-operation batches.
    if (candidateInterface.encodeFunctionData(transact, args) !== data || args[0].length !== 1) fail();
    tx = args[0][0];
  } catch { fail(); }
  const bound = tx.boundParams;
  const unshield = tx.unshieldPreimage;
  if (bound.chainID !== CANDIDATE.chainId || bound.unshield !== 0n
      || bound.adaptContract !== ZeroAddress || bound.adaptParams !== ZeroHash
      || bound.minGasPrice >= (1n << 48n)
      || unshield.npk !== ZeroHash || unshield.value !== 0n
      || unshield.token.tokenType !== 0n || unshield.token.tokenAddress !== ZeroAddress
      || unshield.token.tokenSubID !== 0n) fail();
  if (!scalar(tx.merkleRoot)) fail();
  uniqueScalars(tx.nullifiers); uniqueScalars(tx.commitments);
  if (tx.commitments.length < 2 || bound.commitmentCiphertext.length !== tx.commitments.length) fail();
  const encryptedOutputs = bound.commitmentCiphertext.map(output => {
    // Ciphertext metadata is encrypted by the protocol. This check bounds it;
    // it cannot establish that an opaque memo hides no application identifier.
    if (!bytes(output.annotationData, 1024) || !bytes(output.memo, 1024)
        || output.blindedSenderViewingKey === ZeroHash || output.blindedReceiverViewingKey === ZeroHash
        || output.ciphertext.every(x => x === ZeroHash)) fail();
    return coder.encode([ciphertextType], [output]);
  });
  const boundHash = BigInt(keccak256(coder.encode([boundsType], [bound]))) % FIELD;
  const result = Object.freeze({
    treeNumber: Number(bound.treeNumber),
    stateRoot: tx.merkleRoot,
    nullifiers: Object.freeze([...tx.nullifiers]),
    outputCommitments: Object.freeze([...tx.commitments]),
    encryptedOutputs: Object.freeze(encryptedOutputs),
    proof: coder.encode([proofType], [tx.proof]),
    callData: data,
    publicSignals: Object.freeze([BigInt(tx.merkleRoot), boundHash,
      ...tx.nullifiers.map(BigInt), ...tx.commitments.map(BigInt)]),
    snarkProof: Object.freeze({
      pi_a: Object.freeze([String(tx.proof.a.x), String(tx.proof.a.y), '1']),
      pi_b: Object.freeze([
        Object.freeze([String(tx.proof.b.x[1]), String(tx.proof.b.x[0])]),
        Object.freeze([String(tx.proof.b.y[1]), String(tx.proof.b.y[0])]), Object.freeze(['1', '0']),
      ]),
      pi_c: Object.freeze([String(tx.proof.c.x), String(tx.proof.c.y), '1']),
      protocol: 'groth16', curve: 'bn128',
    }),
    proofVerified: false,
    feeVerified: false,
    ciphertextPrivacyVerified: false,
  });
  qualified.add(result);
  return result;
}

export function isInspectedCandidate(value) { return qualified.has(value); }

/** All fields duplicated in the native public envelope must agree byte-for-byte. */
export function matchCandidateEnvelope(inspected, envelope) {
  if (!isInspectedCandidate(inspected)
      || envelope.schema_version !== 2
      || envelope.domain?.protocol !== CANDIDATE.protocol
      || envelope.domain.chain_id !== Number(CANDIDATE.chainId)
      || envelope.domain.pool_contract !== CANDIDATE.pool
      || envelope.domain.token_contract !== CANDIDATE.token || envelope.domain.token_decimals !== 6
      || envelope.tree_number !== inspected.treeNumber
      || envelope.state_root !== inspected.stateRoot
      || envelope.proof !== inspected.proof || envelope.call_data !== inspected.callData
      || JSON.stringify(envelope.nullifiers) !== JSON.stringify(inspected.nullifiers)
      || JSON.stringify(envelope.output_commitments) !== JSON.stringify(inspected.outputCommitments)
      || JSON.stringify(envelope.encrypted_outputs) !== JSON.stringify(inspected.encryptedOutputs)
      || !inspected.outputCommitments.includes(envelope.fee?.fee_output_commitment)) fail();
  // Token and fee amounts are private circuit values. An ABI match alone does
  // NOT prove them; fee-note decryption plus circuit conservation is mandatory.
  return Object.freeze({ calldataMatched: true, proofVerified: false, feeVerified: false });
}

export function requireProductionPrivateEngine() {
  throw new Error('Private engine unavailable: deployment/artifact pins, real-device qualification and independent privacy review are not approved');
}
