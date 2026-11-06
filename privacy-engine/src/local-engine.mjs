import {CANDIDATE,candidateInterface,poolInterface,inspectCandidateCall,matchCandidateEnvelope} from './candidate-calldata.mjs';
import {verifyCandidateProof,pinnedBytes} from './local-verifier.mjs';
import {provePrepared} from './local-prover.mjs';
import {CompactForest} from './compact-forest.mjs';
import {EncryptedState,unbase64} from './encrypted-state.mjs';
import {captureRecoveryCutoff,scanRecovery,prepareRecoveryBatch} from './recovery-engine.mjs';
import {localKeys,addressData,feeRecipientCommitment,parseUnits,makeTransfer,Transaction,walletFacade,VERSION,CHAIN,TOKEN,TransactNote,getTokenDataERC20,getNoteHash,hex32,OutputType,NO_ADAPT,decryptTransact,decryptShield,tokenGetter} from './protocol.mjs';
import {sha256,getBytes} from 'ethers';
const HASH=/^0x[0-9a-f]{64}$/,SAFE=Number.MAX_SAFE_INTEGER,CHUNK=262144;
const id=()=>{let b=new Uint8Array(32);crypto.getRandomValues(b);return '0x'+Array.from(b,x=>x.toString(16).padStart(2,'0')).join('')};
const num=x=>{if(!Number.isSafeInteger(x)||x<0)throw new Error('INVALID_INPUT');return x};
const quantity=x=>{if(typeof x!=='string'||!/^0x(?:0|[1-9a-f][0-9a-f]*)$/.test(x)||BigInt(x)>BigInt(SAFE))throw new Error('PROTOCOL_MISMATCH');return Number(BigInt(x))};
const canonical=x=>Array.isArray(x)?x.map(canonical):x&&typeof x==='object'?Object.fromEntries(Object.keys(x).sort().map(k=>[k,canonical(x[k])])):x;
const same=(a,b)=>JSON.stringify(canonical(a))===JSON.stringify(canonical(b));
const fields=(o,names)=>{if(!o||typeof o!=='object'||Array.isArray(o)||Object.keys(o).sort().join(',')!==names.sort().join(','))throw new Error('INVALID_INPUT')};
const request=(kind,fields={})=>({schema_version:1,chain_id:42161,kind,...fields});
const wipeRequest=r=>{if(!r)return;for(const n of ['randomIn','valueIn','npkOut','valueOut','leavesIndices'])r.privateInputs[n]?.fill(0n);for(const p of r.privateInputs.pathElements)p.fill(0n);r.privateInputs.nullifyingKey=0n;r.signature?.fill(0n)};

/** One local wallet session. Native journal remains the durable authority for
 * reservations/approval; this cache never marks a transfer successful itself. */
export class LocalPrivateEngine{
 constructor(manifest,port,{prove=provePrepared,clock=()=>Math.floor(Date.now()/1000)}={}){
  this.manifest=manifest;this.port=port;this.proveFunction=prove;this.clock=clock;this.reviews=new Map();this.resetScan();
 }
 resetScan(clearPending=false){this.forest=new CompactForest();this.notes=new Map();if(clearPending||!this.pending)this.pending=new Set();this.cursor=null;this.blockHash=null;this.complete=false;this.dirty=false}
 async rpc(body){const r=await this.port('rpc',body);if(!r||r.schema_version!==1||r.chain_id!==42161||r.kind!==body.kind)throw new Error('PROTOCOL_MISMATCH');return r}
 requireReady(){if(!this.own)throw new Error('UNAVAILABLE')}
 checkDomain(domain){if(!same(domain,this.manifest.domain)||domain.protocol!==CANDIDATE.protocol||domain.chain_id!==42161||domain.pool_contract!==CANDIDATE.pool||domain.token_contract!==TOKEN||domain.token_decimals!==6)throw new Error('PROTOCOL_MISMATCH')}
 async initialize(p){
  fields(p,['wallet_identity','spending_private_key_hex','viewing_private_key_hex','encrypted_state_key_hex','domain','fee_recipient_address','fee_recipient_commitment','creation_block']);
  if(this.own||!HASH.test(p.wallet_identity)||!HASH.test(p.domain?.verifier_digest))throw new Error('INVALID_INPUT');this.checkDomain(p.domain);
  if(p.fee_recipient_address!==this.manifest.fee_recipient_address||p.fee_recipient_commitment!==feeRecipientCommitment(p.fee_recipient_address))throw new Error('PROTOCOL_MISMATCH');
  num(p.creation_block);if(p.creation_block!==this.manifest.deployment_block)throw new Error('PROTOCOL_MISMATCH');
  this.identity=p.wallet_identity;this.own=await localKeys(p.spending_private_key_hex,p.viewing_private_key_hex);
  this.store=new EncryptedState(p.encrypted_state_key_hex,this.identity,this.port);p.spending_private_key_hex='';p.viewing_private_key_hex='';p.encrypted_state_key_hex='';
  const snapshot=await this.store.load();if(snapshot)await this.restoreSnapshot(snapshot);
  return {wallet_identity:this.identity,private_address:this.own.address,chain_id:42161,ready:true};
 }
 async restoreSnapshot(s){
  if(![1,2,3].includes(s.schema_version)||s.wallet_identity!==this.identity||s.private_address!==this.own.address||s.verifier_digest!==this.manifest.domain.verifier_digest||!Array.isArray(s.notes)||s.notes.length>100000||!Array.isArray(s.pending)||s.pending.length>10000)throw new Error('STORAGE');
  const forest=Array.isArray(s.forest)?CompactForest.importLegacy(s.forest,s.notes.map(({tree,position})=>({tree,position}))):CompactForest.import(s.forest),notes=new Map();
  for(const n of s.notes){const note=await TransactNote.deserialize(VERSION,CHAIN,n.serialized,this.own.viewing,tokenGetter);
   const commitment=hex32(note.hash),key=`${num(n.tree)}:${num(n.position)}`,spentHash=n.spent_transaction_hash??null;if(forest.leaf(n.tree,n.position)!==commitment||n.nullifier!==hex32(TransactNote.getNullifier(this.own.nullifyingKey,n.position))||notes.has(key)||typeof n.spent!=='boolean'||!HASH.test(n.transaction_hash)||(spentHash!==null&&(!n.spent||!HASH.test(spentHash))))throw new Error('STORAGE');
   notes.set(key,{tree:n.tree,position:n.position,note,nullifier:n.nullifier,spent:n.spent,spent_transaction_hash:spentHash,block:num(n.block),transaction_hash:n.transaction_hash});}
  if(s.cursor!==null)num(s.cursor);if(s.block_hash!==null&&!HASH.test(s.block_hash))throw new Error('STORAGE');
  if(s.pending.some(n=>!/^([0-9]{1,5}):0x[0-9a-f]{64}$/.test(n)||Number(n.split(':')[0])>65535))throw new Error('STORAGE');
  const pending=new Set([...this.pending,...s.pending]);if(pending.size>10000)throw new Error('STORAGE');
  this.forest=forest;this.notes=notes;this.pending=pending;this.cursor=s.cursor;this.blockHash=s.block_hash;this.complete=false;this.dirty=false;
  // Older unreleased caches did not checkpoint each persisted range or verify
  // complete rollover receipts. Retain every reservation, but rescan their
  // public history before certifying any balance.
  if(s.schema_version<3)this.resetScan();
 }
 snapshot(){return {schema_version:3,wallet_identity:this.identity,private_address:this.own.address,verifier_digest:this.manifest.domain.verifier_digest,cursor:this.cursor,block_hash:this.blockHash,forest:this.forest.export(),notes:[...this.notes.values()].map(n=>({tree:n.tree,position:n.position,serialized:n.note.serialize(),nullifier:n.nullifier,spent:n.spent,spent_transaction_hash:n.spent_transaction_hash??null,block:n.block,transaction_hash:n.transaction_hash})),pending:[...this.pending]}}
 async persist(){await this.store.save(this.snapshot())}
 latestTree(){let latest=0;for(const tree of this.forest.trees.keys())latest=Math.max(latest,tree);return latest}
 async checkpoint(number,expectedHash=null){
  const checkpoint=await this.rpc(request('checkpoint',{number})),tree=this.latestTree();
  if(checkpoint.block?.number!==number||!HASH.test(checkpoint.block?.hash)||(expectedHash!==null&&checkpoint.block.hash!==expectedHash)||checkpoint.tree_number!==tree||checkpoint.next_leaf_index!==this.forest.count(tree)||checkpoint.merkle_root!==this.forest.root(tree))throw new Error('SYNC_INCOMPLETE');
  return checkpoint.block;
 }
 async sync(p,progress=()=>{},targetBlock=null){
  this.requireReady();fields(p,['chain_id']);if(p.chain_id!==42161)throw new Error('INVALID_INPUT');this.complete=false;
  if(this.dirty){const old=await this.store.load();this.resetScan();if(old)await this.restoreSnapshot(old)}
  let head=(await this.rpc(request('head'))).block;num(head.number);if(!HASH.test(head.hash))throw new Error('PROTOCOL_MISMATCH');
  if(targetBlock!==null){num(targetBlock);if(targetBlock>head.number)throw new Error('SYNC_INCOMPLETE');head=(await this.rpc(request('block',{number:targetBlock}))).block;if(head.number!==targetBlock||!HASH.test(head.hash))throw new Error('PROTOCOL_MISMATCH');if(this.cursor!==null&&this.cursor>targetBlock)this.resetScan()}
  if(this.cursor!==null){
   try{if(head.number<this.cursor)throw new Error('SYNC_INCOMPLETE');await this.checkpoint(this.cursor,this.blockHash)}
   catch(e){if(e.message==='SYNC_INCOMPLETE'){this.resetScan();await this.persist()}throw e}
  }
  const initial=this.cursor??(Math.floor(this.manifest.deployment_block/2048)*2048-1);let from=initial+1;
  try{while(from<=head.number){
   // Historical recovery cutoffs are not necessarily the current head. Every
   // request is an aligned power-of-two range, including its partial tail.
   let size=2048;while(from%size!==0||from+size-1>head.number)size/=2;
   const start=from,end=from+size-1;
   const result=await this.rpc(request('logs',{from_block:start,to_block:end}));
   if(result.from_block!==start||result.to_block!==end||!HASH.test(result.block_to_hash)||!Array.isArray(result.logs)||result.logs.length>5000)throw new Error('PROTOCOL_MISMATCH');
   const changed=new Set(),groups=new Map();let previousOrder=-1n;this.dirty=true;
   for(const log of result.logs){
    const block=quantity(log.blockNumber),txindex=quantity(log.transactionIndex),logindex=quantity(log.logIndex),order=(BigInt(block)<<64n)+(BigInt(txindex)<<32n)+BigInt(logindex);
    if(block<start||block>end||log.removed!==false||log.address.toLowerCase()!==CANDIDATE.pool||!HASH.test(log.blockHash)||!HASH.test(log.transactionHash)||order<=previousOrder)throw new Error('PROTOCOL_MISMATCH');previousOrder=order;
    if(!groups.has(block))groups.set(block,[]);groups.get(block).push(log);
   }
   for(const [block,logs] of groups){
    const previousTree=this.latestTree();
    const rolls=logs.some(log=>{const event=poolInterface.parseLog(log);return event&&['Transact','Shield','CommitmentBatch','GeneratedCommitmentBatch'].includes(event.name)&&Number(event.args.treeNumber)>previousTree});
    if(rolls){
     // A known historical root plus a valid rollover inequality cannot prove
     // that the old tree's final few leaves were included. Anchor the prefix
     // immediately before this block, then require complete canonical block
     // receipts to account for every relevant same-block event.
     if(block===0)throw new Error('SYNC_INCOMPLETE');
     await this.checkpoint(block-1);
     const full=await this.rpc(request('block_logs',{number:block}));
     if(full.block?.number!==block||full.block.hash!==logs[0].blockHash||!Array.isArray(full.logs)||!same(full.logs,logs))throw new Error('SYNC_INCOMPLETE');
    }
    for(const log of logs)await this.applyLog(log,changed);
    if(rolls)await this.checkpoint(block,logs[0].blockHash);
   }
   for(const tree of changed){const root=this.forest.root(tree),check=await this.rpc(request('root',{tree_number:tree,root}));if(check.known!==true||check.tree_number!==tree||check.root!==root||check.block.number<end)throw new Error('SYNC_INCOMPLETE')}
   // Persist only a fully checkpointed range. A missing tail must not advance
   // a durable cursor that would otherwise skip that event on every retry.
   const anchor=await this.checkpoint(end,result.block_to_hash);
   this.cursor=end;this.blockHash=anchor.hash;await this.persist();this.dirty=false;from=end+1;
   progress(Math.min(99,Math.floor(100*(end-initial)/Math.max(1,head.number-initial))));
  }
  if(this.cursor!==head.number||this.blockHash!==head.hash)throw new Error('SYNC_INCOMPLETE');
  await this.checkpoint(head.number,head.hash);
  this.complete=true;progress(100);
  return {chain_id:42161,scan_complete:true,scanned_block:this.cursor,canonical_block_hash:this.blockHash};
  }catch(e){this.complete=false;throw e}
 }
 async applyLog(log,changed){
  const parsed=poolInterface.parseLog(log);if(!parsed)throw new Error('PROTOCOL_MISMATCH');const a=parsed.args;
  const tree=Number(a.treeNumber);if(!Number.isSafeInteger(tree)||tree<0||tree>65535)throw new Error('PROTOCOL_MISMATCH');
  if(parsed.name==='Nullified'||parsed.name==='Nullifiers'){
   if(!HASH.test(log.transactionHash))throw new Error('PROTOCOL_MISMATCH');
   const spent=new Set([...a.nullifier].map(hex32));for(const n of this.notes.values())if(n.tree===tree&&spent.has(n.nullifier)){n.spent=true;n.spent_transaction_hash=log.transactionHash;this.pending.delete(`${n.tree}:${n.nullifier}`)}return;
  }
  let hashes,ciphers,preimages;
  if(parsed.name==='Transact'||parsed.name==='CommitmentBatch'){hashes=[...a.hash].map(hex32);ciphers=a.ciphertext}
  else if(parsed.name==='Shield'||parsed.name==='GeneratedCommitmentBatch'){
   preimages=a.commitments;hashes=preimages.map(x=>hex32(getNoteHash(hex32(x.npk),{tokenType:Number(x.token.tokenType),tokenAddress:x.token.tokenAddress,tokenSubID:hex32(x.token.tokenSubID)},x.value)));ciphers=a.shieldCiphertext;
  }else throw new Error('PROTOCOL_MISMATCH');
  const start=Number(a.startPosition);this.forest.append(tree,start,hashes);changed.add(tree);
  for(let i=0;i<hashes.length;i++){
   let note=null;
   if(parsed.name==='Transact')note=await decryptTransact(ciphers[i],hashes[i],this.own);
   else if(parsed.name==='Shield')note=await decryptShield(preimages[i],ciphers[i],this.own);
   else if(parsed.name==='CommitmentBatch')note=await this.decryptLegacy(ciphers[i],hashes[i]);
   else note=await this.decryptLegacyShield(preimages[i],a.encryptedRandom[i],hashes[i]);
   // A commitment may legitimately be funded twice. The leaf position, not
   // the commitment hash, identifies each distinct spendable note.
   if(note&&note.value>0n){this.forest.retain(tree,start+i);this.notes.set(`${tree}:${start+i}`,{tree,position:start+i,note,nullifier:hex32(TransactNote.getNullifier(this.own.nullifyingKey,start+i)),spent:false,block:quantity(log.blockNumber),transaction_hash:log.transactionHash})}
  }
 }
 async decryptLegacy(cipher,hash){return decryptLegacyTransact(cipher,hash,this.own)}
 async decryptLegacyShield(preimage,encryptedRandom,hash){return decryptLegacyDeposit(preimage,encryptedRandom,hash,this.own)}
 available(){return [...this.notes.values()].filter(n=>!n.spent&&!this.pending.has(`${n.tree}:${n.nullifier}`))}
 async balance(p){this.requireReady();fields(p,['chain_id','token_contract']);if(p.chain_id!==42161||p.token_contract!==TOKEN)throw new Error('INVALID_INPUT');return {chain_id:42161,token_contract:TOKEN,units:this.available().reduce((a,n)=>a+n.note.value,0n).toString(),scan_complete:this.complete,scanned_block:this.cursor??0,canonical_block_hash:this.blockHash??'0x'+'0'.repeat(64)}}
 async checkUnspent(selected){
  if(!selected.length||selected.length>11)throw new Error('CONSOLIDATION_REQUIRED');const nullifiers=selected.map(n=>n.nullifier);while(nullifiers.length<16){const pad=id();if(!nullifiers.includes(pad))nullifiers.push(pad)}
  const r=await this.rpc(request('nullifiers',{tree_number:selected[0].tree,nullifiers}));
  if(!same(r.nullifiers,nullifiers)||!Array.isArray(r.spent)||r.spent.length!==16||r.spent.some(v=>typeof v!=='boolean')||r.tree_number!==selected[0].tree||r.block.number<(this.cursor??0))throw new Error('PROTOCOL_MISMATCH');
  if(r.spent.slice(0,selected.length).some(Boolean))throw new Error('SYNC_INCOMPLETE');
 }
 async prepare(p){
  this.requireReady();fields(p,['intent_id','chain_id','token_contract','recipient','amount_units','maximum','fee_schedule']);
  if(!HASH.test(p.intent_id)||p.chain_id!==42161||p.token_contract!==TOKEN||typeof p.maximum!=='boolean'||(p.maximum?p.amount_units!==null:p.amount_units===null))throw new Error('INVALID_INPUT');
  if(!this.complete)throw new Error('SYNC_INCOMPLETE');addressData(p.recipient);if(this.reviews.size>=4)throw new Error('UNAVAILABLE');
  const f=p.fee_schedule;this.checkDomain(f.domain);
  if(f.fee_recipient_address!==this.manifest.fee_recipient_address||f.fee_recipient_commitment!==feeRecipientCommitment(f.fee_recipient_address)||!HASH.test(f.schedule_id)||num(f.valid_from)>this.clock()||num(f.valid_until)<=this.clock())throw new Error('QUOTE_EXPIRED');
  const fee=parseUnits(f.fee_units);if(fee===0n)throw new Error('INVALID_INPUT');const available=this.available(),byTree=new Map();
  for(const n of available){if(!byTree.has(n.tree))byTree.set(n.tree,[]);byTree.get(n.tree).push(n)}
  let selected,amount,total;
  if(p.maximum){if(byTree.size!==1||available.length>11)throw new Error('CONSOLIDATION_REQUIRED');selected=available;total=selected.reduce((a,n)=>a+n.note.value,0n);amount=total-fee}
  else{amount=parseUnits(p.amount_units);for(const group of byTree.values()){let sum=0n,choice=[];for(const n of group){choice.push(n);sum+=n.note.value;if(sum>=amount+fee)break;if(choice.length===11)break}if(sum>=amount+fee&&choice.length<=11){selected=choice;total=sum;break}}}
  if(!selected?.length||amount<=0n||total<amount+fee)throw new Error('INSUFFICIENT_FUNDS');
  const shape=String(selected.length).padStart(2,'0')+'x03';if(!this.manifest.circuits[shape])throw new Error('CONSOLIDATION_REQUIRED');
  await this.checkUnspent(selected);const change=total-amount-fee;
  const recipientNote=makeTransfer(p.recipient,this.own,amount),feeNote=makeTransfer(f.fee_recipient_address,this.own,fee,OutputType.BroadcasterFee),changeNote=makeTransfer(this.own.address,this.own,change,OutputType.Change);
  const outputs=[recipientNote,feeNote,changeNote];for(let i=outputs.length-1;i>0;i--){const b=crypto.getRandomValues(new Uint32Array(1))[0];const j=b%(i+1);[outputs[i],outputs[j]]=[outputs[j],outputs[i]]}
  const tx=new Transaction(CHAIN,getTokenDataERC20(TOKEN),selected[0].tree,selected.map(n=>({note:n.note,position:n.position})),outputs,NO_ADAPT);
  const req=await tx.generateTransactionRequest(walletFacade(this.own,this.forest),VERSION,'',{minGasPrice:0n});
  const feeTerms={schedule_id:f.schedule_id,fee_units:f.fee_units,fee_recipient_commitment:f.fee_recipient_commitment,fee_output_commitment:hex32(feeNote.hash),max_native_fee_wei:f.max_native_fee_wei,gas_limit:f.gas_limit,quote_valid_until:f.valid_until};
  const intent={schema_version:2,intent_id:p.intent_id,tree_number:selected[0].tree,domain:this.manifest.domain,recipient:p.recipient,amount_units:amount.toString(),fee:feeTerms,input_notes:selected.map(n=>({commitment:hex32(n.note.hash),nullifier:n.nullifier,units:n.note.value.toString()})),state_root:hex32(req.publicInputs.merkleRoot),output_commitments:req.publicInputs.commitmentsOut.map(hex32),recipient_output_commitment:hex32(recipientNote.hash),change_units:change.toString(),change_output_commitment:change===0n?null:hex32(changeNote.hash),padding_output_commitments:change===0n?[hex32(changeNote.hash)]:[],created_at:this.clock(),approval_digest:''};
  const review=id();this.reviews.set(review,{intent,request:req,selected});
  return {review_id:review,intent_json:JSON.stringify(intent),note_values:intent.input_notes.map(n=>n.units),service_fee_units:f.fee_units,protocol_fee_units:'0',recipient_units:amount.toString(),scan_complete:true,includes_all_spendable_notes:selected.length===available.length,needs_consolidation:false,quote_valid_until:f.valid_until};
 }
 async artifact(spec){
  if(!spec||typeof spec.path!=='string'||!/^artifacts\/[0-9]{2}x[0-9]{2}\.(wasm|zkey|vkey\.json)$/.test(spec.path)||!/^[0-9a-f]{64}$/.test(spec.sha256)||!Number.isSafeInteger(spec.bytes)||spec.bytes<=0||spec.bytes>512*1024*1024)throw new Error('UNAVAILABLE');
  const out=new Uint8Array(spec.bytes);try{for(let offset=0;offset<out.length;){const r=await this.port('artifact_read',{path:spec.path,offset,length:Math.min(CHUNK,out.length-offset)});const b=unbase64(r.base64);if(r.offset!==offset||r.total_bytes!==out.length||r.sha256!==spec.sha256||!b.length||b.length>CHUNK||offset+b.length>out.length)throw new Error('PROTOCOL_MISMATCH');out.set(b,offset);offset+=b.length;b.fill(0)}return pinnedBytes(out,spec.sha256,512*1024*1024)}finally{out.fill(0)}
 }
 async loadArtifacts(shape){const c=this.manifest.circuits[shape];if(!c)throw new Error('UNAVAILABLE');const [wasm,zkey,vkey]=await Promise.all(['wasm','zkey','vkey'].map(x=>this.artifact(c[x]).then(bytes=>({...c[x],bytes}))));return {wasm,zkey,vkey}}
 async prove(p,progress=()=>{}){
  fields(p,['review_id','intent_json']);const r=this.reviews.get(p.review_id);if(!r)throw new Error('INVALID_INPUT');const approved=JSON.parse(p.intent_json),unsigned={...approved,approval_digest:''};
  if(!HASH.test(approved.approval_digest)||!same(unsigned,r.intent)||this.clock()>=approved.fee.quote_valid_until)throw new Error('QUOTE_EXPIRED');
  await this.checkUnspent(r.selected);const root=await this.rpc(request('root',{tree_number:r.selected[0].tree,root:r.intent.state_root}));if(root.known!==true)throw new Error('SYNC_INCOMPLETE');
  const signature=walletFacade(this.own,this.forest).sign(r.request.publicInputs);r.request.signature=[...signature.R8,signature.S];
  try{
   const inspected=await this.proveFunction(r.request,s=>this.loadArtifacts(s),progress);
   if(!same(inspected.nullifiers,approved.input_notes.map(n=>n.nullifier))||!same(inspected.outputCommitments,approved.output_commitments)||inspected.stateRoot!==approved.state_root)throw new Error('PROOF_INVALID');
   const envelope={schema_version:2,domain:approved.domain,tree_number:approved.tree_number,state_root:inspected.stateRoot,nullifiers:[...inspected.nullifiers],output_commitments:[...inspected.outputCommitments],encrypted_outputs:[...inspected.encryptedOutputs],proof:inspected.proof,call_data:inspected.callData,fee:approved.fee,operation_digest:''};
   for(const n of r.selected)this.pending.add(`${n.tree}:${n.nullifier}`);await this.persist();return {envelope_json:JSON.stringify(envelope),verified:true};
  }finally{wipeRequest(r.request);this.reviews.delete(p.review_id)}
 }
 async verify(p){fields(p,['envelope_json']);const e=JSON.parse(p.envelope_json);this.checkDomain(e.domain);const inspected=inspectCandidateCall({to:CANDIDATE.pool,value:0n,data:e.call_data});matchCandidateEnvelope(inspected,e);
  const shape=String(e.nullifiers.length).padStart(2,'0')+'x'+String(e.output_commitments.length).padStart(2,'0'),c=this.manifest.circuits[shape];if(!c)throw new Error('PROOF_INVALID');
  if(!await verifyCandidateProof(inspected,await this.artifact(c.vkey),c.vkey.sha256))throw new Error('PROOF_INVALID');return {verified:true};
 }
 async observe(p){
  fields(p,['envelope_json','transaction_hash','prior_receipt_json']);await this.verify({envelope_json:p.envelope_json});const e=JSON.parse(p.envelope_json);
  if(p.prior_receipt_json){const old=JSON.parse(p.prior_receipt_json),b=(await this.rpc(request('block',{number:old.block_number}))).block;if(b.hash!==old.block_hash){const h=(await this.rpc(request('head'))).block;return {observation_type:'reorg',observation_json:JSON.stringify({block_number:old.block_number,previous_block_hash:old.block_hash,canonical_block_hash:b.hash,finalized_height:h.number})}}}
  let transactionHash=p.transaction_hash;
  if(transactionHash===null){
   // Lost acknowledgements do not require a trusted operator response. Discover
   // a candidate in the full public-pool scan, then verify the exact operation.
   await this.sync({chain_id:42161});
   const spent=[...this.notes.values()].filter(n=>n.tree===e.tree_number&&e.nullifiers.includes(n.nullifier)&&n.spent);
   if(spent.length!==e.nullifiers.length)return {observation_type:'none',observation_json:null};
   const candidates=new Set(spent.map(n=>n.spent_transaction_hash));
   if(candidates.size!==1||!HASH.test([...candidates][0]))return {observation_type:'none',observation_json:null};
   transactionHash=[...candidates][0];
  }
  if(!HASH.test(transactionHash))throw new Error('INVALID_INPUT');
  const r=await this.rpc(request('receipt',{transaction_hash:transactionHash}));if(r.receipt===null)return {observation_type:'none',observation_json:null};const receipt=r.receipt;
  if(receipt.transactionHash!==transactionHash||r.transaction_input!==e.call_data)throw new Error('PROOF_INVALID');
  const blockNumber=quantity(receipt.blockNumber),canonical=(await this.rpc(request('block',{number:blockNumber}))).block;
  if(receipt.blockHash!==canonical.hash)throw new Error('SYNC_INCOMPLETE');const success=quantity(receipt.status)===1;
  if(quantity(receipt.status)>1)throw new Error('PROTOCOL_MISMATCH');
  const inspected=inspectCandidateCall({to:CANDIDATE.pool,value:0n,data:e.call_data});let nullifiers=[],commitments=[];
  for(const log of receipt.logs){if(log.address.toLowerCase()!==CANDIDATE.pool)continue;const parsed=poolInterface.parseLog(log);if(parsed?.name==='Nullified'&&Number(parsed.args.treeNumber)===inspected.treeNumber)nullifiers.push(...parsed.args.nullifier);if(parsed?.name==='Transact')commitments.push(...parsed.args.hash)}
  if(success&&(!same(nullifiers,e.nullifiers)||!same(commitments,e.output_commitments)))throw new Error('PROOF_INVALID');if(!success&&(nullifiers.length||commitments.length))throw new Error('PROOF_INVALID');
  const observation={operation_digest:e.operation_digest,chain_id:42161,pool_contract:CANDIDATE.pool,transaction_hash:transactionHash,block_hash:receipt.blockHash,canonical_block_hash:canonical.hash,block_number:blockNumber,finalized_height:num(r.finalized_block.number),execution_success:success,nullifiers:success?e.nullifiers:[],output_commitments:success?e.output_commitments:[],fee_units_charged:success?e.fee.fee_units:'0',native_fee_wei:(BigInt(receipt.gasUsed)*BigInt(receipt.effectiveGasPrice)).toString()};
  return {observation_type:'receipt',observation_json:JSON.stringify(observation)};
 }
 async exportState(){this.requireReady();await this.persist();return {saved:true,scanned_block:this.cursor??0}}
 async recovery_cutoff(p){return captureRecoveryCutoff(this,p)}
 async recovery_scan(p,progress){return scanRecovery(this,p,progress)}
 async recovery_prepare(p){return prepareRecoveryBatch(this,p)}
 async restore_reservations(p){
  this.requireReady();fields(p,['reservations']);if(!Array.isArray(p.reservations)||p.reservations.length>10000)throw new Error('INVALID_INPUT');
  const merged=new Set(this.pending);for(const n of p.reservations){fields(n,['tree_number','nullifier']);if(num(n.tree_number)>65535||!HASH.test(n.nullifier))throw new Error('INVALID_INPUT');merged.add(`${n.tree_number}:${n.nullifier}`)}
  if(merged.size>10000)throw new Error('STORAGE');this.pending=merged;await this.persist();return {restored:true};
 }
 async release_cancelled_reservations(p){
  // Only the native coordinator can invoke this after an authenticated journal
  // cancellation. Released, backup-exposed and imported proofs cannot cancel.
  this.requireReady();fields(p,['tree_number','nullifiers']);if(num(p.tree_number)>65535||!Array.isArray(p.nullifiers)||!p.nullifiers.length||p.nullifiers.length>16||p.nullifiers.some(n=>!HASH.test(n))||new Set(p.nullifiers).size!==p.nullifiers.length)throw new Error('INVALID_INPUT');
  const before=new Set(this.pending);let changed=false;for(const n of p.nullifiers)changed=this.pending.delete(`${p.tree_number}:${n}`)||changed;
  if(!changed)return {released:true};
  try{await this.persist()}catch(e){this.pending=before;throw e}return {released:true};
 }
 async importState(){this.requireReady();const snapshot=await this.store.load();if(!snapshot)throw new Error('STORAGE');await this.restoreSnapshot(snapshot);return {loaded:true,scan_complete:false}}
 async cancel(p){fields(p,['review_id']);const r=this.reviews.get(p.review_id);if(r)wipeRequest(r.request);this.reviews.delete(p.review_id);return {cancelled:true}}
 close(){for(const r of this.reviews.values())wipeRequest(r.request);this.reviews.clear();this.own?.destroy();this.own=null;this.store?.destroy();this.resetScan(true);return {closed:true}}
}
// Legacy encrypted and generated commitments use the SDK's compatibility
// primitives. They must remain in scans so restoration never silently drops
// spendable notes from earlier contract versions.
import legacyKeys from '../node_modules/@railgun-community/engine/dist/utils/keys-utils-legacy.js';
import memoModule from '../node_modules/@railgun-community/engine/dist/note/memo.js';
async function decryptLegacyTransact(c,hash,own){let shared;try{
 const sender=getBytes(hex32(c.ephemeralKeys[0])),receiver=getBytes(hex32(c.ephemeralKeys[1]));shared=await legacyKeys.getSharedSymmetricKeyLegacy(own.viewing,sender);if(!shared)return null;
 const raw=c.ciphertext.map(x=>hex32(x).slice(2)),memo=c.memo.map(x=>hex32(x).slice(2)),count=memoModule.LEGACY_MEMO_METADATA_BYTE_CHUNKS;
 const n=await TransactNote.decrypt(VERSION,CHAIN,own.data,{iv:raw[0].slice(0,32),tag:raw[0].slice(32),data:raw.slice(1)},shared,memo.slice(count).join(''),memo.slice(0,count).join(''),own.viewing,receiver,sender,false,true,tokenGetter,undefined,undefined);
 return hex32(n.hash)===hash?n:null;
}catch{return null}finally{shared?.fill(0)}}
async function decryptLegacyDeposit(p,random,hash,own){try{
 const first=hex32(random[0]).slice(2),second=BigInt(random[1]).toString(16).padStart(32,'0');
 const n=await TransactNote.deserialize(VERSION,CHAIN,{npk:hex32(p.npk).slice(2),encryptedRandom:[first,second],tokenHash:hex32(p.token.tokenAddress).slice(2),value:BigInt(p.value).toString(16).padStart(32,'0'),recipientAddress:own.address,memoField:[]},own.viewing,tokenGetter);
 return hex32(n.hash)===hash?n:null;
}catch{return null}}
