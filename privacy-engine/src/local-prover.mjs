import {groth16} from 'snarkjs';
import {Prover,hex32} from './protocol.mjs';
import {candidateInterface,inspectCandidateCall,CANDIDATE} from './candidate-calldata.mjs';
import {verifyCandidateProof,pinnedBytes} from './local-verifier.mjs';
import {ZeroAddress,ZeroHash} from 'ethers';
/** No global SDK proof cache: upstream cache keys contain the private witness. */
export async function provePrepared(request,loadArtifacts,progress=()=>{}){
 const p=request.publicInputs,v=request.privateInputs;
 const shape=String(p.nullifiers.length).padStart(2,'0')+'x'+String(p.commitmentsOut.length).padStart(2,'0');
 const a=await loadArtifacts(shape);
 const wasm=pinnedBytes(a.wasm.bytes,a.wasm.sha256,64*1024*1024),zkey=pinnedBytes(a.zkey.bytes,a.zkey.sha256,512*1024*1024);
 const formatted={merkleRoot:p.merkleRoot,boundParamsHash:p.boundParamsHash,nullifiers:p.nullifiers,commitmentsOut:p.commitmentsOut,
 token:v.tokenAddress,publicKey:v.publicKey,signature:request.signature,randomIn:v.randomIn,valueIn:v.valueIn,pathElements:v.pathElements.flat(2),leavesIndices:v.leavesIndices,nullifyingKey:v.nullifyingKey,npkOut:v.npkOut,valueOut:v.valueOut};
 progress(10);
 try{
  const {proof,publicSignals}=await groth16.fullProve(formatted,wasm,zkey,undefined,{sanityCheck:true},{singleThread:true});
  const expected=[p.merkleRoot,p.boundParamsHash,...p.nullifiers,...p.commitmentsOut].map(String);
  if(JSON.stringify(publicSignals)!==JSON.stringify(expected))throw new Error('PROOF_INVALID');
  const tx={proof:Prover.formatProof(proof),merkleRoot:hex32(p.merkleRoot),nullifiers:p.nullifiers.map(hex32),commitments:p.commitmentsOut.map(hex32),boundParams:request.boundParams,unshieldPreimage:{npk:ZeroHash,token:{tokenType:0,tokenAddress:ZeroAddress,tokenSubID:0n},value:0n}};
  const data=candidateInterface.encodeFunctionData('transact',[[tx]]);
  const inspected=inspectCandidateCall({to:CANDIDATE.pool,value:0n,data});progress(90);
  if(!await verifyCandidateProof(inspected,a.vkey.bytes,a.vkey.sha256))throw new Error('PROOF_INVALID');
  progress(100);return inspected;
 }finally{wasm.fill(0);zkey.fill(0);formatted.pathElements.fill(0n)}
}
