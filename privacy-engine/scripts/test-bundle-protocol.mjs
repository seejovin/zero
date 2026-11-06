#!/usr/bin/env node
/** Full packaged-engine test through its transferred MessagePort. This uses
 * real SDK ciphertext, native approval and real Groth16 with public fixtures;
 * pool observations are offline fixtures and no mainnet transaction is sent. */
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';
import {MessageChannel} from 'node:worker_threads';
import {readFile,writeFile,mkdir,copyFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {resolve,join} from 'node:path';
import assert from 'node:assert/strict';
import {sha256} from 'ethers';
import {localKeys,makeTransfer,Transaction,walletFacade,NO_ADAPT,VERSION,CHAIN,hex32,getTokenDataERC20,TOKEN,feeRecipientCommitment} from '../src/protocol.mjs';
import {MerkleForest} from '../src/merkle-forest.mjs';
import {CANDIDATE,poolInterface,candidateInterface} from '../src/candidate-calldata.mjs';
import {verifyProviderEnvelope} from '../src/provider-verifier.mjs';

const [artifactArg,bindingArg]=process.argv.slice(2);
if(!artifactArg||!bindingArg)throw Error('Usage: test-bundle-protocol.mjs QUALIFIED_ARTIFACT_DIRECTORY GENERATED_BINDING_DIRECTORY');
const engineRoot=resolve(new URL('..',import.meta.url).pathname),root=resolve(engineRoot,'..'),artifacts=resolve(artifactArg),bindings=resolve(bindingArg),out=join(root,'build/private-engine-vm-protocol');
await mkdir(join(out,'artifacts'),{recursive:true});
const hash=x=>'0x'+x.repeat(64).slice(0,64),now=Math.floor(Date.now()/1000);
function native(request){const p=spawnSync('python3',[join(root,'scripts/private-protocol-fixture.py'),bindings],{input:JSON.stringify(request),encoding:'utf8',timeout:30000,maxBuffer:1048576});if(p.status!==0)throw Error(p.stderr.slice(0,1000));return JSON.parse(p.stdout)}
const local=native({operation:'derive'}),own=await localKeys(local.spending_private_key_hex,local.viewing_private_key_hex),fee=await localKeys('21'.repeat(32),'22'.repeat(32)),recipient=await localKeys('31'.repeat(32),'32'.repeat(32));
const qualified=JSON.parse(await readFile(join(artifacts,'qualified-circuits.json'),'utf8'));
for(const entry of Object.values(qualified.circuits['01x03']))await copyFile(join(artifacts,entry.path),join(out,entry.path));
const domain={protocol:CANDIDATE.protocol,chain_id:42161,pool_contract:CANDIDATE.pool,token_contract:TOKEN,token_decimals:6,verifier_digest:'0x'+qualified.circuits['01x03'].vkey.sha256};
const manifest={schema_version:1,approved:false,domain,fee_recipient_address:fee.address,deployment_block:0,circuits:{'01x03':qualified.circuits['01x03']}};
await writeFile(join(out,'fixture-manifest.json'),JSON.stringify(manifest));
const built=spawnSync(process.execPath,[join(engineRoot,'scripts/build-browser.mjs'),out,join(out,'fixture-manifest.json')],{cwd:engineRoot,encoding:'utf8'});if(built.status!==0)throw Error(built.stderr);
const source=makeTransfer(fee.address,fee,1000001n),incoming=makeTransfer(own.address,fee,1000001n),sourceForest=new MerkleForest(),forest=new MerkleForest();sourceForest.append(0,0,[hex32(source.hash)]);forest.append(0,0,[hex32(incoming.hash)]);
const transaction=new Transaction(CHAIN,getTokenDataERC20(TOKEN),0,[{note:source,position:0}],[incoming],NO_ADAPT),request=await transaction.generateTransactionRequest(walletFacade(fee,sourceForest),VERSION,'',{minGasPrice:0n});
const event=poolInterface.encodeEventLog(poolInterface.getEvent('Transact'),[0,0,[hex32(incoming.hash)],request.boundParams.commitmentCiphertext]);
const log={...event,address:CANDIDATE.pool,blockNumber:'0x1',blockHash:hash('4'),transactionHash:hash('5'),transactionIndex:'0x0',logIndex:'0x0',removed:false},block={number:1,hash:hash('4'),parent_hash:hash('3'),timestamp:now};
let head=block,poolLogs=[log],receipt=null,transactionInput=null;
const blocks=new Map([[1,block]]),roots=new Set([forest.root(0)]);
const checkpoints=new Map([[1,{tree_number:0,next_leaf_index:forest.count(0),merkle_root:forest.root(0)}]]);
let listener,networkAttempts=0,snapshot=null,upload=null;const portNames=new Set();
async function handlePort(name,p){portNames.add(name);
 if(name==='storage_read')return snapshot?{exists:true,base64:Buffer.from(snapshot.subarray(p.offset,p.offset+p.length)).toString('base64'),offset:p.offset,total_bytes:snapshot.length,sha256:sha256(snapshot).slice(2)}:{exists:false,base64:null,offset:0,total_bytes:0,sha256:null};
 if(name==='storage_write'){
  if(p.action==='begin'){upload=new Uint8Array(p.total_bytes);return {upload_id:hash('8'),offset:0}}
  if(p.action==='write'){const bytes=Buffer.from(p.base64,'base64');upload.set(bytes,p.offset);return {upload_id:hash('8'),offset:p.offset+bytes.length}}
  if(p.action==='commit'){snapshot=upload;return {committed:true,total_bytes:snapshot.length,sha256:sha256(snapshot).slice(2)}}
  if(p.action==='abort'){upload=null;return {aborted:true}}
 }
 if(name==='artifact_read'){const bytes=await readFile(join(artifacts,p.path));return {base64:bytes.subarray(p.offset,p.offset+p.length).toString('base64'),offset:p.offset,total_bytes:bytes.length,sha256:sha256(bytes).slice(2)}}
 if(name==='rpc'){
  const base={schema_version:1,chain_id:42161,kind:p.kind};
  if(p.kind==='head')return {...base,block:head};
  if(p.kind==='block')return {...base,block:blocks.get(p.number)};
  if(p.kind==='checkpoint')return {...base,block:blocks.get(p.number),...checkpoints.get(p.number)};
  if(p.kind==='logs')return {...base,from_block:p.from_block,to_block:p.to_block,block_to_hash:blocks.get(p.to_block).hash,logs:poolLogs.filter(l=>Number(BigInt(l.blockNumber))>=p.from_block&&Number(BigInt(l.blockNumber))<=p.to_block)};
  if(p.kind==='root'){assert.equal(roots.has(p.root),true);return {...base,tree_number:p.tree_number,root:p.root,known:true,block:head}}
  if(p.kind==='nullifiers')return {...base,tree_number:p.tree_number,nullifiers:p.nullifiers,spent:Array(16).fill(false),block:head};
  if(p.kind==='receipt')return {...base,receipt,transaction_input:transactionInput,finalized_block:head};
 }
 throw Error('Unexpected packaged-engine port');
}
const sandbox={crypto:webcrypto,TextEncoder,TextDecoder,Uint8Array,Uint32Array,ArrayBuffer,DataView,WebAssembly,URL,Blob,ReadableStream,WritableStream,TransformStream,MessageChannel,atob,btoa,setTimeout,clearTimeout,setInterval,clearInterval,queueMicrotask,performance,navigator:{product:'Gecko',hardwareConcurrency:1},location:{origin:'https://private.local-wallet.invalid',href:'https://private.local-wallet.invalid/index.html'},console:{log(){},warn(){},error(){}},addEventListener(kind,f){if(kind==='message')listener=f},fetch(){networkAttempts++;throw Error('NETWORK_DISABLED')}};
sandbox.window=sandbox;sandbox.self=sandbox;sandbox.globalThis=sandbox;
const context=vm.createContext(sandbox,{codeGeneration:{strings:false,wasm:true}});vm.runInContext(await readFile(join(out,'engine.js'),'utf8'),context,{timeout:15000});
const channel=new MessageChannel(),pending=new Map(),session=hash('7');let nextId=16,resolveReady;
const ready=new Promise(r=>resolveReady=r);
channel.port1.on('message',async value=>{
 const m=JSON.parse(value);
 if(m.kind==='ready')resolveReady(m);
 else if(m.kind==='port_request'){
  try{const result=await handlePort(m.port,m.payload);channel.port1.postMessage(JSON.stringify({schema_version:1,kind:'port_result',session_id:session,request_id:m.request_id,port_request_id:m.port_request_id,result}))}
  catch{channel.port1.postMessage(JSON.stringify({schema_version:1,kind:'port_error',session_id:session,request_id:m.request_id,port_request_id:m.port_request_id,code:'UNAVAILABLE'}))}
 }else if(m.kind==='result'||m.kind==='error'){pending.get(m.request_id)?.(m);pending.delete(m.request_id)}
});
const bounded=promise=>Promise.race([promise,new Promise((_,reject)=>{const t=setTimeout(()=>reject(Error('Packaged engine timeout')),120000);t.unref()})]);
const call=async(command,payload,expectedError=null)=>{const id='0x'+(nextId++).toString(16).padStart(64,'0');const response=await bounded(new Promise(resolve=>{pending.set(id,resolve);channel.port1.postMessage(JSON.stringify({schema_version:1,kind:'request',session_id:session,request_id:id,command,payload}))}));if(expectedError!==null){assert.equal(response.kind,'error',JSON.stringify(response));assert.equal(response.code,expectedError);return response}assert.equal(response.kind,'result',JSON.stringify(response));return response.result};
listener({data:JSON.stringify({schema_version:1,kind:'connect',session_id:session}),ports:[channel.port2],origin:sandbox.location.origin});await bounded(ready);
try{
 assert.equal((await call('initialize',{...local,domain,fee_recipient_address:fee.address,fee_recipient_commitment:feeRecipientCommitment(fee.address),creation_block:0})).private_address,own.address);
 await call('restore_reservations',{reservations:[]});await call('sync',{chain_id:42161});const balance=await call('balance',{chain_id:42161,token_contract:TOKEN});assert.equal(balance.units,'1000001');assert.equal(balance.scan_complete,true);
 const schedule={domain,schedule_id:hash('2'),fee_units:'7',fee_recipient_address:fee.address,fee_recipient_commitment:feeRecipientCommitment(fee.address),max_native_fee_wei:'1000000000000000',gas_limit:1000000,valid_from:now-1,valid_until:now+600};
 const prepared=await call('prepare',{intent_id:hash('6'),chain_id:42161,token_contract:TOKEN,recipient:recipient.address,amount_units:null,maximum:true,fee_schedule:schedule});
 const sealed=native({operation:'seal_intent',intent_json:prepared.intent_json});
 const start=Date.now();process.stderr.write('Proving through the actual packaged MessagePort engine.\n');const proof=await call('prove',{review_id:prepared.review_id,intent_json:sealed.intent_json});const milliseconds=Date.now()-start;
 const envelope=native({operation:'seal_envelope',envelope_json:proof.envelope_json});await call('verify',envelope);
 const config={verifier_digest:domain.verifier_digest,circuits:manifest.circuits,fee_recipient_address:fee.address,fee_viewing_private_key_hex:'22'.repeat(32),additional_native_fee_cap_wei:'0'};
 await verifyProviderEnvelope(JSON.parse(envelope.envelope_json),config,p=>readFile(join(artifacts,p)));
 assert.equal((await call('balance',{chain_id:42161,token_contract:TOKEN})).units,'0');
 const outgoingHash=hash('9'),observedPayload={...envelope,transaction_hash:outgoingHash,prior_receipt_json:null};
 assert.deepEqual(await call('observe',observedPayload),{observation_type:'none',observation_json:null});
 // The gateway's acknowledgement is lost. The engine must discover the
 // nullifiers through its pool scan, then match exact calldata and outputs.
 const sealedEnvelope=JSON.parse(envelope.envelope_json),decoded=candidateInterface.decodeFunctionData('transact',sealedEnvelope.call_data)[0][0];
 head={number:2,hash:hash('a'),parent_hash:block.hash,timestamp:now+1};blocks.set(2,head);
 const events=[poolInterface.encodeEventLog(poolInterface.getEvent('Nullified'),[0,sealedEnvelope.nullifiers]),poolInterface.encodeEventLog(poolInterface.getEvent('Transact'),[0,1,sealedEnvelope.output_commitments,decoded.boundParams.commitmentCiphertext])];
 const outgoingLogs=events.map((e,i)=>({...e,address:CANDIDATE.pool,blockNumber:'0x2',blockHash:head.hash,transactionHash:outgoingHash,transactionIndex:'0x0',logIndex:'0x'+i.toString(16),removed:false}));
 poolLogs.push(...outgoingLogs);forest.append(0,1,sealedEnvelope.output_commitments);roots.add(forest.root(0));
 checkpoints.set(2,{tree_number:0,next_leaf_index:forest.count(0),merkle_root:forest.root(0)});
 receipt={transactionHash:outgoingHash,blockHash:head.hash,blockNumber:'0x2',status:'0x1',gasUsed:'0x61a80',effectiveGasPrice:'0x5f5e100',logs:outgoingLogs};transactionInput=sealedEnvelope.call_data;
 const observed=await call('observe',{...observedPayload,transaction_hash:null});assert.equal(observed.observation_type,'receipt');
 const receiptEvidence=JSON.parse(observed.observation_json);assert.equal(receiptEvidence.execution_success,true);assert.equal(receiptEvidence.transaction_hash,outgoingHash);assert.deepEqual(receiptEvidence.nullifiers,sealedEnvelope.nullifiers);assert.equal(receiptEvidence.fee_units_charged,'7');
 transactionInput='0x';await call('observe',observedPayload,'PROOF_INVALID');transactionInput=sealedEnvelope.call_data;
 receipt={...receipt,logs:[outgoingLogs[0]]};await call('observe',observedPayload,'PROOF_INVALID');
 await call('close',{});assert.equal(networkAttempts,0);
 const result={schema_version:1,passed:true,actual_packaged_engine:true,transferred_message_port:true,sdk_encrypted_note_scanned:true,real_groth16_proof_generated:true,proof_verified_in_bundle:true,provider_fee_verified:true,native_approval_crosscheck:true,encrypted_chunked_persistence:true,input_reserved_after_proof:true,lost_acknowledgement_reconciled_from_pool:true,missing_receipt_preserves_uncertainty:true,wrong_calldata_and_missing_output_logs_rejected:true,no_string_eval:true,network_requests:0,ports:[...portNames].sort(),proof_milliseconds:milliseconds,browser_csp_tested:false,android_webview_tested:false,deployed_verifier_correspondence_checked:false,mainnet_funds_used:false,production_enabled:false};
 await writeFile(join(out,'verification.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result));
}finally{channel.port1.close();channel.port2.close();own.destroy();fee.destroy();recipient.destroy()}
process.exit(0);
