// Offline ABI fixture generator using the pinned SDK/ethers independently of
// the Rust event checker. Groth16 points/ciphertexts are deliberately invalid.
// NEVER broadcast transactions constructed from this fixture.
import {writeFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {candidateInterface,inspectCandidateCall,CANDIDATE} from '../../privacy-engine/src/candidate-calldata.mjs';
const require=createRequire(import.meta.url);
const {ZeroAddress,ZeroHash}=require('../../privacy-engine/node_modules/ethers');
const hash=n=>'0x'+BigInt(n).toString(16).padStart(64,'0');
const transaction={
 proof:{a:{x:1n,y:2n},b:{x:[3n,4n],y:[5n,6n]},c:{x:7n,y:8n}},
 merkleRoot:hash(20),nullifiers:[hash(21),hash(22)],commitments:[hash(23),hash(24)],
 boundParams:{treeNumber:1,minGasPrice:0,unshield:0,chainID:42161,adaptContract:ZeroAddress,adaptParams:ZeroHash,
  commitmentCiphertext:[0,1].map(n=>({ciphertext:[hash(30+n),hash(31),hash(32),hash(33)],
   blindedSenderViewingKey:hash(40),blindedReceiverViewingKey:hash(41),annotationData:'0x0102',memo:'0x'}))},
 unshieldPreimage:{npk:ZeroHash,token:{tokenType:0,tokenAddress:ZeroAddress,tokenSubID:0},value:0},
};
const call_data=candidateInterface.encodeFunctionData('transact',[[transaction]]);
const inspected=inspectCandidateCall({to:CANDIDATE.pool,value:0n,data:call_data});
const nullified=candidateInterface.encodeEventLog(candidateInterface.getEvent('Nullified'),[1,transaction.nullifiers]);
const transact=candidateInterface.encodeEventLog(candidateInterface.getEvent('Transact'),[1,17,transaction.commitments,transaction.boundParams.commitmentCiphertext]);
const fixture={warning:'INVALID PROOF. OFFLINE ABI FIXTURE ONLY. NEVER BROADCAST.',
 tree_number:1,state_root:inspected.stateRoot,nullifiers:inspected.nullifiers,output_commitments:inspected.outputCommitments,
 encrypted_outputs:inspected.encryptedOutputs,proof:inspected.proof,call_data,selector:call_data.slice(0,10),logs:[nullified,transact],
 view_selectors:Object.fromEntries(['rootHistory','nullifiers','getVerificationKey','treeNumber','nextLeafIndex','merkleRoot'].map(name=>[name,candidateInterface.getFunction(name).selector]))};
await writeFile(new URL('./abi-case.json',import.meta.url),JSON.stringify(fixture,null,2)+'\n');
