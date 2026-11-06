import {test} from 'node:test';import assert from 'node:assert/strict';
import {localKeys,makeTransfer,Transaction,walletFacade,NO_ADAPT,VERSION,CHAIN,hex32,getTokenDataERC20,TOKEN,decryptTransact,feeRecipientCommitment,OutputType,ShieldNote} from '../src/protocol.mjs';
import {MerkleForest} from '../src/merkle-forest.mjs';
import {LocalPrivateEngine} from '../src/local-engine.mjs';
import {candidateInterface,poolInterface,CANDIDATE} from '../src/candidate-calldata.mjs';
import {EncryptedState} from '../src/encrypted-state.mjs';
import {sha256} from 'ethers';
import sdkMerkletree from '../node_modules/@railgun-community/engine/dist/merkletree/merkletree.js';
const h=x=>'0x'+x.repeat(64).slice(0,64),own=await localKeys('11'.repeat(32),'12'.repeat(32)),payee=await localKeys('21'.repeat(32),'22'.repeat(32));
function storagePort(){let saved=null,upload=null;return async(port,p)=>{
 if(port==='storage_write'){switch(p.action){case'begin':upload={id:h('8'),bytes:new Uint8Array(p.total_bytes),hash:p.sha256,offset:0};return{upload_id:upload.id,offset:0};case'write':{assert.equal(p.offset,upload.offset);const b=Buffer.from(p.base64,'base64');upload.bytes.set(b,p.offset);upload.offset+=b.length;return{upload_id:upload.id,offset:upload.offset}}case'commit':assert.equal(sha256(upload.bytes).slice(2),upload.hash);saved=upload.bytes;return{committed:true,total_bytes:saved.length,sha256:upload.hash};case'abort':upload=null;return{aborted:true}}}
 if(port==='storage_read')return saved?{exists:true,base64:Buffer.from(saved.subarray(p.offset,p.offset+p.length)).toString('base64'),offset:p.offset,total_bytes:saved.length,sha256:sha256(saved).slice(2)}:{exists:false,base64:null,offset:0,total_bytes:0,sha256:null};throw Error('unexpectedport');};}
const domain={protocol:CANDIDATE.protocol,chain_id:42161,pool_contract:CANDIDATE.pool,token_contract:TOKEN,token_decimals:6,verifier_digest:h('1')};
const manifest={domain,fee_recipient_address:payee.address,deployment_block:0,circuits:{'01x03':{},'02x03':{}}};
const fee={domain,schedule_id:h('2'),fee_units:'7',fee_recipient_address:payee.address,fee_recipient_commitment:feeRecipientCommitment(payee.address),max_native_fee_wei:'1000000000000000',gas_limit:1000000,valid_from:1,valid_until:9999999999};
const init=()=>({wallet_identity:h('3'),spending_private_key_hex:'11'.repeat(32),viewing_private_key_hex:'12'.repeat(32),encrypted_state_key_hex:'13'.repeat(32),domain,fee_recipient_address:payee.address,fee_recipient_commitment:feeRecipientCommitment(payee.address),creation_block:0});
test('SDK encrypted transfer hides sender; wrong key and altered ciphertext fail',async()=>{
 const input=makeTransfer(own.address,own,100n),output=makeTransfer(payee.address,own,100n),f=new MerkleForest();f.append(0,0,[hex32(input.hash)]);
 const tx=new Transaction(CHAIN,getTokenDataERC20(TOKEN),0,[{note:input,position:0}],[output],NO_ADAPT);const r=await tx.generateTransactionRequest(walletFacade(own,f),VERSION,'',{minGasPrice:0n});
 const c=r.boundParams.commitmentCiphertext[0];const n=await decryptTransact(c,hex32(output.hash),payee);assert.equal(n.value,100n);assert.equal(n.getSenderAddress(),undefined);assert.equal(n.memoText,undefined);
 assert.equal(await decryptTransact(c,hex32(output.hash),own),null);
 const tamper={...c,ciphertext:[...c.ciphertext]};tamper.ciphertext[1]=h('9');assert.equal(await decryptTransact(tamper,hex32(output.hash),payee),null);
 assert.equal(await decryptTransact(c,h('9'),payee),null);
});
test('append-only Merkle tree rejects gaps/overwrites; paths and reloaded roots agree',()=>{
 const f=new MerkleForest();f.append(0,0,[1n,2n,3n].map(hex32));for(let i=0;i<3;i++)assert.equal(MerkleForest.checkProof(hex32(i+1),i,f.proof(0,i),f.root(0)),true);
 assert.throws(()=>f.append(0,1,[hex32(9)]));assert.throws(()=>f.append(0,4,[hex32(9)]));assert.equal(MerkleForest.import(f.export()).root(0),f.root(0));
 assert.equal(MerkleForest.checkProof(hex32(4),2,f.proof(0,2),f.root(0)),false);
});
test('local Merkle roots agree with the actual upstream SDK across batch boundaries',async()=>{
 const sdk=new sdkMerkletree.Merkletree(null,CHAIN,VERSION,async()=>true,1000),f=new MerkleForest(),levels=[];
 for(const [start,values] of [[0,[1n,2n,3n]],[3,[4n,5n,6n,7n,8n]]]){
  const first=[];first[0]=[];values.forEach((v,i)=>first[0][start+i]=hex32(v).slice(2));
  const result=await sdkMerkletree.Merkletree.fillHashWriteGroup(first,0,start,start+values.length,async(level,index)=>levels[level]?.[index]??sdk.zeros[level]);
  for(let level=0;level<result.length;level++){levels[level]??=[];for(const index of Object.keys(result[level]))levels[level][index]=result[level][index]}
  f.append(0,start,values.map(hex32));assert.equal(f.root(0).slice(2),result[16][0]);
 }
});
test('encrypted chunked snapshot authenticates wallet identity and key',async()=>{
 const port=storagePort();const state=new EncryptedState('31'.repeat(32),h('4'),port);const snapshot={secret:'recipient-private',large:'x'.repeat(400000)};await state.save(snapshot);assert.deepEqual(await state.load(),snapshot);
 await assert.rejects(new EncryptedState('32'.repeat(32),h('4'),port).load(),/STORAGE/);await assert.rejects(new EncryptedState('31'.repeat(32),h('5'),port).load(),/STORAGE/);
});
async function fundedEngine(amounts=[1000001n]){
 const storage=storagePort(),forest=new MerkleForest(),inputs=amounts.map(v=>makeTransfer(own.address,own,v));forest.append(0,0,inputs.map(n=>hex32(n.hash)));
 const engine=new LocalPrivateEngine(manifest,async(port,p)=>{if(port!=='rpc')return storage(port,p);if(p.kind==='nullifiers')return{schema_version:1,chain_id:42161,kind:p.kind,tree_number:0,nullifiers:p.nullifiers,spent:Array(16).fill(false),block:{number:1,hash:h('4')}};if(p.kind==='root')return{schema_version:1,chain_id:42161,kind:p.kind,tree_number:0,root:p.root,known:true,block:{number:1,hash:h('4')}};throw Error('unexpected rpc')});await engine.initialize(init());engine.forest=forest;engine.complete=true;engine.cursor=1;engine.blockHash=h('4');inputs.forEach((note,position)=>engine.notes.set(hex32(note.hash),{note,position,tree:0,nullifier:hex32(TransactNote.getNullifier(engine.own.nullifyingKey,position)),spent:false,block:1,transaction_hash:h('5')}));return engine;
}
import {TransactNote} from '../src/protocol.mjs';
test('Max uses exact integer balance minus fee and creates zero-valued private padding',async()=>{
 const e=await fundedEngine([1000001n,999999n]);const r=await e.prepare({intent_id:h('6'),chain_id:42161,token_contract:TOKEN,recipient:payee.address,amount_units:null,maximum:true,fee_schedule:fee});const i=JSON.parse(r.intent_json);
 assert.equal(i.amount_units,'1999993');assert.equal(i.change_units,'0');assert.equal(i.padding_output_commitments.length,1);assert.equal(i.output_commitments.length,3);assert.equal(r.includes_all_spendable_notes,true);assert.equal(i.input_notes.reduce((s,n)=>s+BigInt(n.units),0n),BigInt(i.amount_units)+BigInt(i.fee.fee_units));
 await assert.rejects(e.prove({review_id:r.review_id,intent_json:JSON.stringify({...i,amount_units:'1',approval_digest:h('7')})}),/QUOTE_EXPIRED/);e.close();
});
test('Max refuses a partial spend across trees and sync-incomplete state',async()=>{
 const e=await fundedEngine();e.complete=false;await assert.rejects(e.prepare({intent_id:h('6'),chain_id:42161,token_contract:TOKEN,recipient:payee.address,amount_units:null,maximum:true,fee_schedule:fee}),/SYNC_INCOMPLETE/);e.complete=true;
 const n=makeTransfer(own.address,own,9n);e.notes.set(hex32(n.hash),{note:n,tree:1,position:0,nullifier:h('8'),spent:false});await assert.rejects(e.prepare({intent_id:h('6'),chain_id:42161,token_contract:TOKEN,recipient:payee.address,amount_units:null,maximum:true,fee_schedule:fee}),/CONSOLIDATION_REQUIRED/);e.close();
});
test('duplicate funded commitments remain distinct notes through encryption, scan and restore',async()=>{
 const storage=storagePort(),e=new LocalPrivateEngine(manifest,storage);await e.initialize(init());
 const input=makeTransfer(own.address,own,100n),output=makeTransfer(own.address,payee,80n),f=new MerkleForest();f.append(0,0,[hex32(input.hash)]);
 const tx=new Transaction(CHAIN,getTokenDataERC20(TOKEN),0,[{note:input,position:0}],[output],NO_ADAPT),r=await tx.generateTransactionRequest(walletFacade(own,f),VERSION,'',{minGasPrice:0n});
 const encrypted=poolInterface.encodeEventLog(poolInterface.getEvent('Transact'),[0,0,[hex32(output.hash),hex32(output.hash)],[r.boundParams.commitmentCiphertext[0],r.boundParams.commitmentCiphertext[0]]]);
 await e.applyLog({...encrypted,blockNumber:'0x1',transactionHash:h('5')},new Set());
 assert.equal(e.notes.size,2);assert.notEqual(e.notes.get('0:0').nullifier,e.notes.get('0:1').nullifier);assert.equal((await e.balance({chain_id:42161,token_contract:TOKEN})).units,'160');
 await e.persist();const loaded=new LocalPrivateEngine(manifest,storage);await loaded.initialize(init());assert.equal(loaded.notes.size,2);assert.equal((await loaded.balance({chain_id:42161,token_contract:TOKEN})).units,'160');
 const reservation={tree_number:0,nullifier:loaded.notes.get('0:0').nullifier};assert.deepEqual(await loaded.restore_reservations({reservations:[reservation]}),{restored:true});
 loaded.resetScan();assert.equal(loaded.pending.has(`0:${reservation.nullifier}`),true);await loaded.importState();assert.equal((await loaded.balance({chain_id:42161,token_contract:TOKEN})).units,'80');
 await assert.rejects(loaded.restore_reservations({reservations:[{...reservation,tree_number:65536}]}),/INVALID_INPUT/);e.close();loaded.close();
});
test('cancelled reservations release only their inputs and survive restart; failed persistence restores the reservation',async()=>{
 const storage=storagePort();let failCommit=false;
 const port=async(name,p)=>{if(failCommit&&name==='storage_write'&&p.action==='commit')throw Error('STORAGE');return storage(name,p)};
 const e=new LocalPrivateEngine(manifest,port);await e.initialize(init());
 const first={tree_number:0,nullifier:h('a')},second={tree_number:1,nullifier:h('a')};
 await e.restore_reservations({reservations:[first,second]});
 failCommit=true;await assert.rejects(e.release_cancelled_reservations({tree_number:0,nullifiers:[first.nullifier]}),/STORAGE/);
 assert.equal(e.pending.has(`0:${first.nullifier}`),true);assert.equal(e.pending.has(`1:${second.nullifier}`),true);
 failCommit=false;await e.release_cancelled_reservations({tree_number:0,nullifiers:[first.nullifier]});
 // Cleanup is safe to repeat after a crash between cache commit and UI update.
 failCommit=true;assert.deepEqual(await e.release_cancelled_reservations({tree_number:0,nullifiers:[first.nullifier]}),{released:true});failCommit=false;
 const reopened=new LocalPrivateEngine(manifest,port);await reopened.initialize(init());
 assert.equal(reopened.pending.has(`0:${first.nullifier}`),false);assert.equal(reopened.pending.has(`1:${second.nullifier}`),true);
 e.close();reopened.close();
});
