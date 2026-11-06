#!/usr/bin/env node
/** Actual Groth16 proof and fee verification over public unfunded fixture keys.
 * Pool and balance observations are controlled offline fixtures, not mainnet. */
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {generateKeyPairSync,sign,createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import {sha256,getBytes} from 'ethers';
import {LocalPrivateEngine} from '../src/local-engine.mjs';
import {localKeys,makeTransfer,TransactNote,hex32,TOKEN,feeRecipientCommitment,decryptTransact} from '../src/protocol.mjs';
import {candidateInterface,CANDIDATE,inspectCandidateCall} from '../src/candidate-calldata.mjs';
import {verifyCandidateProof} from '../src/local-verifier.mjs';
import {verifyProviderEnvelope} from '../src/provider-verifier.mjs';

const [artifactArg,bindingArg,shape='01x03']=process.argv.slice(2);
if(!artifactArg||!bindingArg||process.argv.length>5||!/^(0[1-9]|1[01])x03$/.test(shape))throw Error('Usage: test-protocol.mjs QUALIFIED_ARTIFACT_DIRECTORY GENERATED_BINDING_DIRECTORY [01x03..11x03]');
const root=resolve(new URL('../..',import.meta.url).pathname),directory=resolve(artifactArg),bindings=resolve(bindingArg);
const report=JSON.parse(await readFile(join(directory,'qualified-circuits.json'),'utf8'));
if(!report.circuits[shape])throw Error('Requested circuit is not qualified');
const inputCount=Number(shape.slice(0,2)),expectedRecipient=1000001n*BigInt(inputCount)-7n;
const h=n=>'0x'+n.repeat(64).slice(0,64),now=Math.floor(Date.now()/1000);
function native(request){const p=spawnSync('python3',[join(root,'scripts/private-protocol-fixture.py'),bindings],{input:JSON.stringify(request),encoding:'utf8',maxBuffer:1048576,timeout:30000});if(p.status!==0)throw Error(`Native fixture failed: ${p.stderr.slice(0,1200)}`);return JSON.parse(p.stdout)}
const local=native({operation:'derive'});
assert.equal(local.spending_private_key_hex,'08b2d974aa7fffd9d068b78c34434c534ddcd9343fcbf5aa12cf78e1a3c1ccb9');
assert.equal(local.viewing_private_key_hex,'9a9e1ca3b9476dc8500b43f30f34104c92a3eedfd727757ffd0ad15da8e11572');
const recipient=await localKeys('31'.repeat(32),'32'.repeat(32)),provider=await localKeys('21'.repeat(32),'22'.repeat(32));
const domain={protocol:CANDIDATE.protocol,chain_id:42161,pool_contract:CANDIDATE.pool,token_contract:TOKEN,token_decimals:6,verifier_digest:'0x'+report.circuits['01x03'].vkey.sha256};
const manifest={schema_version:1,approved:false,domain,fee_recipient_address:provider.address,deployment_block:0,circuits:report.circuits};
const schedule={domain,schedule_id:h('2'),fee_units:'7',fee_recipient_address:provider.address,fee_recipient_commitment:feeRecipientCommitment(provider.address),max_native_fee_wei:'1000000000000000',gas_limit:1000000,valid_from:now-1,valid_until:now+600};
// Real Ed25519 packet -> native verifier, including exact serde byte ordering.
const signer=generateKeyPairSync('ed25519'),publicKey=signer.publicKey.export({format:'der',type:'spki'}).subarray(-32);
const packet={schema_version:1,schedule,key_id:'0x'+createHash('sha256').update(publicKey).digest('hex'),signature:'0x'+sign(null,Buffer.from('local-wallet/shared-fee-schedule/v1\0'+JSON.stringify(schedule)),signer.privateKey).toString('hex')};
const verifiedSchedule=JSON.parse(native({operation:'verify_schedule',packet_json:JSON.stringify(packet),public_key:publicKey.toString('hex'),now}).schedule_json);assert.deepEqual(verifiedSchedule,schedule);
let snapshot=null,upload=null;
const engine=new LocalPrivateEngine(manifest,async(port,p)=>{
 if(port==='storage_read')return snapshot?{exists:true,base64:Buffer.from(snapshot.subarray(p.offset,p.offset+p.length)).toString('base64'),offset:p.offset,total_bytes:snapshot.length,sha256:sha256(snapshot).slice(2)}:{exists:false,base64:null,offset:0,total_bytes:0,sha256:null};
 if(port==='storage_write'){
  if(p.action==='begin'){upload=new Uint8Array(p.total_bytes);return {upload_id:h('8'),offset:0}}
  if(p.action==='write'){const b=Buffer.from(p.base64,'base64');upload.set(b,p.offset);return {upload_id:h('8'),offset:p.offset+b.length}}
  if(p.action==='commit'){snapshot=upload;return {committed:true,total_bytes:snapshot.length,sha256:sha256(snapshot).slice(2)}}
  if(p.action==='abort'){upload=null;return {aborted:true}}
 }
 if(port==='artifact_read'){const bytes=await readFile(join(directory,p.path));return {base64:bytes.subarray(p.offset,p.offset+p.length).toString('base64'),offset:p.offset,total_bytes:bytes.length,sha256:sha256(bytes).slice(2)}}
 if(port==='rpc'&&p.kind==='nullifiers')return {schema_version:1,chain_id:42161,kind:p.kind,tree_number:0,nullifiers:p.nullifiers,spent:Array(16).fill(false),block:{number:1,hash:h('4')}};
 if(port==='rpc'&&p.kind==='root')return {schema_version:1,chain_id:42161,kind:p.kind,tree_number:0,root:p.root,known:true,block:{number:1,hash:h('4')}};
 throw Error('Unexpected qualification port');
});
await engine.initialize({...local,domain,fee_recipient_address:provider.address,fee_recipient_commitment:schedule.fee_recipient_commitment,creation_block:0});
const inputs=Array.from({length:inputCount},()=>makeTransfer(engine.own.address,engine.own,1000001n));engine.forest.append(0,0,inputs.map(n=>hex32(n.hash)));
inputs.forEach((note,position)=>{engine.forest.retain(0,position);engine.notes.set(`0:${position}`,{note,tree:0,position,nullifier:hex32(TransactNote.getNullifier(engine.own.nullifyingKey,position)),spent:false,block:1,transaction_hash:h('5')})});engine.cursor=1;engine.blockHash=h('4');engine.complete=true;
const prepared=await engine.prepare({intent_id:h('6'),chain_id:42161,token_contract:TOKEN,recipient:recipient.address,amount_units:null,maximum:true,fee_schedule:verifiedSchedule});
const intentJson=native({operation:'seal_intent',intent_json:prepared.intent_json}).intent_json;
const started=Date.now();process.stderr.write(`Generating actual ${shape} Groth16 proof with pinned published parameters.\n`);
const proof=await engine.prove({review_id:prepared.review_id,intent_json:intentJson});
const elapsed=Date.now()-started,envelopeJson=native({operation:'seal_envelope',envelope_json:proof.envelope_json}).envelope_json,envelope=JSON.parse(envelopeJson);
assert.deepEqual(await engine.verify({envelope_json:envelopeJson}),{verified:true});
const config={verifier_digest:domain.verifier_digest,circuits:report.circuits,fee_recipient_address:provider.address,fee_viewing_private_key_hex:'22'.repeat(32),additional_native_fee_cap_wei:'0'};
const checked=await verifyProviderEnvelope(envelope,config,p=>readFile(join(directory,p)));assert.equal(checked.operation_digest,envelope.operation_digest);
const tx=candidateInterface.decodeFunctionData('transact',envelope.call_data)[0][0],intent=JSON.parse(intentJson),recipientIndex=envelope.output_commitments.indexOf(intent.recipient_output_commitment);
const received=await decryptTransact(tx.boundParams.commitmentCiphertext[recipientIndex],intent.recipient_output_commitment,recipient);assert.equal(received.value,expectedRecipient);assert.equal(received.getSenderAddress(),undefined);
const wrongProvider={...config,fee_viewing_private_key_hex:'23'.repeat(32)};await assert.rejects(verifyProviderEnvelope(envelope,wrongProvider,p=>readFile(join(directory,p))));
await assert.rejects(verifyProviderEnvelope({...envelope,fee:{...envelope.fee,fee_units:'8'}},config,p=>readFile(join(directory,p))));
// Decode into a mutable tuple tree and change a public nullifier. The retained
// proof must fail against the changed public inputs, not merely its JSON shape.
const mutable=tx.toArray(true);mutable[2][0]=h('1');const changed=inspectCandidateCall({to:CANDIDATE.pool,value:0n,data:candidateInterface.encodeFunctionData('transact',[[mutable]])});
assert.equal(await verifyCandidateProof(changed,await readFile(join(directory,report.circuits[shape].vkey.path)),report.circuits[shape].vkey.sha256),false);
const journal=native({operation:'journal',intent_json:intentJson,envelope_json:envelopeJson});assert.equal(journal.phase,'unknown');
const evidence={schema_version:1,passed:true,actual_protocol_proof_generated:true,actual_protocol_proof_verified:true,provider_fee_decrypted_and_bound:true,recipient_ciphertext_decrypted:true,hidden_sender_checked:true,native_key_derivation_matches_sdk:true,native_ed25519_fee_packet_verified:true,native_approval_and_envelope_binding:true,encrypted_journal_exact_unknown_replay:true,tampered_public_input_rejected:true,wrong_fee_and_key_rejected:true,exact_max_units:expectedRecipient.toString(),service_fee_units:'7',circuit:shape,proof_milliseconds:elapsed,
 proving_parameters_sha256:report.circuits[shape].zkey.sha256,verification_key_sha256:report.circuits[shape].vkey.sha256,network_requests:0,canonical_pool_observations:'controlled offline fixtures',deployed_verifier_correspondence_checked:false,mainnet_funds_used:false,android_runtime_tested:false,production_enabled:false};
const evidenceDirectory=join(root,'build',shape==='01x03'?'private-protocol-test':`private-protocol-test-${shape}`);await mkdir(evidenceDirectory,{recursive:true});
await writeFile(join(evidenceDirectory,'verification.json'),JSON.stringify(evidence,null,2)+'\n');
await writeFile(join(evidenceDirectory,'envelope.json'),envelopeJson+'\n');
await writeFile(join(evidenceDirectory,'intent.json'),intentJson+'\n');
await writeFile(join(evidenceDirectory,'provider-config.json'),JSON.stringify({...config,artifact_directory:directory},null,2)+'\n',{mode:0o600});
engine.close();recipient.destroy();provider.destroy();process.stdout.write(JSON.stringify(evidence)+'\n');process.exit(0);
