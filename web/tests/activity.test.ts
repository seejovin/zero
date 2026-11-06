import { test } from "node:test";
import assert from "node:assert/strict";
import { IDBFactory } from "fake-indexeddb";
import { Journal } from "../src/journal";
import { ActivityReader } from "../src/activity";
import type { ActivityProof } from "../src/types";
import { fixtureCore, freshJournal, FROM, vault, prepare, sign, networks, blockHash } from "./fixtures";
import { activityProof, moveProof, ACTIVITY_HASH } from "./activity-fixtures";

async function activityEvents(journal:Journal):Promise<unknown[]> {return new Promise((resolve,reject)=>{const r=(journal as any).db.transaction("activity_events").objectStore("activity_events").getAll();r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});}
async function mutate(journal:Journal,storeName:string,key:string,fn:(row:any)=>void):Promise<void> {return new Promise((resolve,reject)=>{const tx=(journal as any).db.transaction(storeName,"readwrite");const store=tx.objectStore(storeName),r=store.get(key);r.onsuccess=()=>{fn(r.result);store.put(r.result);};tx.oncomplete=resolve;tx.onabort=()=>reject(tx.error);});}
function reader(journal:Journal,proof:ActivityProof,overrides:Record<string,unknown>={}) {
  const p=proof as any;
  const network=networks.find((n:any)=>n.chain_id===8453);
  const rpc={network,connectionInfo:()=>({provider:"Fixture"}),verifyChain:async()=>{},activityCandidates:async()=>{throw new Error("Index unavailable");},
    block:async(tag:string)=>tag==="finalized"?{...p.canonical_block,number:`0x${proof.finalized_height.toString(16)}`}:{...p.canonical_block,transactions:tag==="latest"?p.canonical_block.transactions:[]},
    read:async(method:string)=>method==="eth_getTransactionByHash"?p.transaction:method==="eth_getTransactionReceipt"?p.receipt:[],
    assertCanonical:async()=>{},...overrides};
  // Canonical receipt verification needs transaction membership; use the real block for numbered reads.
  if(!overrides.block)rpc.block=async(tag:string)=>tag==="finalized"?{...p.canonical_block,number:`0x${proof.finalized_height.toString(16)}`}:{...p.canonical_block};
  return new ActivityReader(fixtureCore,journal,rpc as never);
}

test("the v0.2 database migration preserves the exact vault, signature, nonce reservation and events",async()=>{
  const source=await freshJournal();const p=prepare("migration",8453),s=sign(p);await source.journal.recordPrepared(p);await source.journal.recordSigned(s);const payment=await source.journal.get(s.intent_id),events=await source.journal.events(s.intent_id);
  const factory=new IDBFactory(),name=crypto.randomUUID();
  await new Promise<void>((resolve,reject)=>{
    const open=factory.open(name,1);open.onupgradeneeded=()=>{const db=open.result;db.createObjectStore("vault",{keyPath:"id"});const payments=db.createObjectStore("payments",{keyPath:"intent_id"});payments.createIndex("account","account");payments.createIndex("nonce","nonce_key",{unique:true});const events=db.createObjectStore("events",{keyPath:"id",autoIncrement:true});events.createIndex("intent","intent_id");db.createObjectStore("snapshots",{keyPath:"key"});db.createObjectStore("latest",{keyPath:"account"});};
    open.onsuccess=()=>{const db=open.result,tx=db.transaction(["vault","payments","events"],"readwrite");tx.objectStore("vault").add(vault);tx.objectStore("payments").add(payment);for(const event of events)tx.objectStore("events").add(event);tx.oncomplete=()=>{db.close();resolve();};tx.onabort=()=>reject(tx.error);};open.onerror=()=>reject(open.error);
  });
  const migrated=await Journal.open(fixtureCore,factory,name);
  assert.deepEqual(await migrated.vault(),vault);assert.deepEqual(await migrated.get(s.intent_id),payment);assert.deepEqual(await migrated.events(s.intent_id),events);assert.deepEqual(await migrated.listActivity(8453,FROM),[]);
  await migrated.recordActivity(8453,FROM,activityProof());assert.equal((await migrated.listActivity(8453,FROM))[0].amount,"0.01");assert.equal((await migrated.get(s.intent_id)).signed?.raw_transaction,s.raw_transaction);
});
test("incoming observations are idempotent, reloadable and always backed by shared WASM validation",async()=>{
  const {journal,factory,name}=await freshJournal(),proof=activityProof();const first=await journal.recordActivity(8453,FROM,proof);await journal.recordActivity(8453,FROM,proof);
  assert.equal((await activityEvents(journal)).length,1);assert.equal(first[0].direction,"received");assert.equal(first[0].amount,"0.01");journal.close();
  const reopened=await Journal.open(fixtureCore,factory,name);assert.deepEqual(await reopened.listActivity(8453,FROM),first);assert.deepEqual(await reopened.listActivity(1,FROM),[]);
  await mutate(reopened,"activities",first[0].id,row=>{row.activity.amount_units="999999";});await assert.rejects(reopened.listActivity(8453,FROM),/verified receipt/);
});
test("many matching events cannot multiply a bounded RPC proof into unbounded local writes",async()=>{
  const {journal}=await freshJournal(),proof=activityProof();
  (proof.canonical_block as any).provider_metadata="x".repeat(250_000);
  const log=(proof.receipt as any).logs[0];
  (proof.receipt as any).logs=Array.from({length:20},(_,i)=>({...log,logIndex:`0x${i.toString(16)}`}));
  await assert.rejects(journal.recordActivity(8453,FROM,proof),/too much activity evidence/);
  assert.deepEqual(await journal.listActivity(8453,FROM),[]);assert.equal((await activityEvents(journal)).length,0);
});
test("a later receipt cannot add or remove financial events at an unchanged transaction block",async()=>{
  const first=await freshJournal(),proof=activityProof();await first.journal.recordActivity(8453,FROM,proof);
  const added=structuredClone(proof);const logs=(added.receipt as any).logs;logs.push({...logs[0],logIndex:"0xa"});
  await assert.rejects(first.journal.recordActivity(8453,FROM,added),/changes the transfer set/);assert.equal((await first.journal.listActivity(8453,FROM)).length,1);
  const second=await freshJournal();await second.journal.recordActivity(8453,FROM,added);
  await assert.rejects(second.journal.recordActivity(8453,FROM,proof),/changes the transfer set/);assert.equal((await second.journal.listActivity(8453,FROM)).length,2);
});
test("activity re-inclusion requires explicit canonical contradiction, and finality is monotonic",async()=>{
  const {journal}=await freshJournal(),proof=activityProof(),replacement=moveProof(proof);const [a]=await journal.recordActivity(8453,FROM,proof);
  await assert.rejects(journal.recordActivity(8453,FROM,replacement),/explicitly invalidated/);await assert.rejects(journal.invalidateActivity(8453,FROM,a.id,a.block_hash,a.block_hash),/Canonical/);
  await journal.invalidateActivity(8453,FROM,a.id,a.block_hash,blockHash("22"));assert.equal((await journal.listActivity(8453,FROM))[0].inclusion,"reorged");
  const [b]=await journal.recordActivity(8453,FROM,replacement);assert.equal(b.id,a.id);assert.equal(b.block_hash,blockHash("22"));assert.equal(b.inclusion,"included");
  replacement.finalized_height=100;await journal.recordActivity(8453,FROM,replacement);replacement.finalized_height=1;const [kept]=await journal.recordActivity(8453,FROM,replacement);assert.equal(kept.inclusion,"finalized");assert.equal((await activityEvents(journal)).length,4);
});
test("a finalized contradiction remains an incident even when a replacement changes log ordinal",async()=>{
  const {journal}=await freshJournal(),proof=activityProof(8453,"USDC",100);const [a]=await journal.recordActivity(8453,FROM,proof);
  await journal.invalidateActivity(8453,FROM,a.id,a.block_hash,blockHash("22"));const replacement=moveProof(proof);const r=replacement.receipt as any;
  const unrelated={...r.logs[0],address:"0x2222222222222222222222222222222222222222",logIndex:"0x1"};r.logs.unshift(unrelated);
  const kept=await journal.recordActivity(8453,FROM,replacement);assert.equal(kept.length,1);assert.equal(kept[0].inclusion,"reorged");assert.equal(kept[0].block_hash,a.block_hash);assert.equal((await journal.listActivity(8453,FROM)).length,1);
  const result=await reader(journal,replacement).lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"unavailable");assert.equal(result.items[0].inclusion,"reorged");
});
test("manual lookup distinguishes received, pending, absent, unrelated and reverted without losing history",async()=>{
  const {journal}=await freshJournal(),proof=activityProof();let result=await reader(journal,proof).lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"received");assert.equal(result.items[0].amount,"0.01");
  result=await reader(journal,proof,{read:async()=>null}).lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"not_found");assert.equal(result.items.length,1);assert.match(result.error!,/does not prove/);
  const pending={...(proof.transaction as any),blockHash:null,blockNumber:null};result=await reader(journal,proof,{read:async(method:string)=>method==="eth_getTransactionByHash"?pending:null}).lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"pending");assert.equal(result.items.length,1);
  const other=await freshJournal(),failed=activityProof();(failed.receipt as any).status="0x0";(failed.receipt as any).logs=[];result=await reader(other.journal,failed).lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"failed");assert.deepEqual(result.items,[]);
  const unrelated=activityProof();(unrelated.receipt as any).logs=[];result=await reader(other.journal,unrelated).lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"unrelated");
});
test("lookup rejects another hash or a canonical block change and never relabels read failure as transfer failure",async()=>{
  const {journal}=await freshJournal(),proof=activityProof();
  const wrong=reader(journal,proof,{read:async(method:string)=>method==="eth_getTransactionByHash"?{...(proof.transaction as any),hash:blockHash("bb")}:proof.receipt});
  let result=await wrong.lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"unavailable");assert.match(result.error!,/another transaction/);assert.deepEqual(result.items,[]);
  result=await reader(journal,proof,{assertCanonical:async()=>{throw new Error("chain changed");}}).lookup(FROM,ACTIVITY_HASH);assert.equal(result.lookup_status,"unavailable");assert.deepEqual(await journal.listActivity(8453,FROM),[]);
});
test("recent token scan persists 0.01 receipt evidence, reports partial history and keeps unresolved candidates retryable",async()=>{
  const {journal}=await freshJournal(),proof=activityProof();let windows=0;
  const read=async(method:string,params:any[])=>{
    if(method==="eth_getLogs") {windows++;const f=params[0];assert.ok(BigInt(f.toBlock)-BigInt(f.fromBlock)<2000n);return [(proof.receipt as any).logs[0]];}
    return method==="eth_getTransactionByHash"?proof.transaction:proof.receipt;
  };
  let result=await reader(journal,proof,{read}).refresh(FROM);assert.equal(result.items[0].amount,"0.01");assert.equal(result.coverage,"partial");assert.equal(result.complete,false);assert.equal(windows,2);assert.equal(await journal.activityCursor(8453,FROM),100);
  const empty=await freshJournal();result=await reader(empty.journal,proof,{read:async(method:string)=>method==="eth_getLogs"?[(proof.receipt as any).logs[0]]:null}).refresh(FROM);assert.equal(await empty.journal.activityCursor(8453,FROM),null);assert.ok(result.warnings.some(w=>w.includes("could not yet be verified")));
});
