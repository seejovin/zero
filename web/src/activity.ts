import type { ActivityProof, ActivityResult, ChainActivity, CoreApi } from "./types";
import { Journal } from "./journal";
import { NetworkClient } from "./network";
import { address, hash, hexQuantity, quantity, safeNumber, sameAddress } from "./units";

const TRANSFER="0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const MAX_CANDIDATES=24;
const RANGE=2_000;
const MAX_BLOCKS=8_000;
const errorText=(error:unknown)=>error instanceof Error?error.message:"The network activity could not be read.";
type Observation={status:"observed"|"pending"|"not_found"|"failed";proof?:ActivityProof;items:ChainActivity[];warnings:string[]};

/** Candidate indexes are hints only. Financial activity comes exclusively from
 * the shared Rust validator's canonical transaction/receipt observations. */
export class ActivityReader {
  constructor(private core:CoreApi,private journal:Journal,private rpc:NetworkClient) {}
  private result(items:ChainActivity[],coverage:ActivityResult["coverage"],warnings:string[],error:string|null,from:number|null=null,to:number|null=null):ActivityResult {
    return {items,coverage,complete:false,from_block:from,to_block:to,provider:this.rpc.connectionInfo().provider,observed_at:Math.floor(Date.now()/1000),warnings,error};
  }
  private async correctPrior(account:string,items:ChainActivity[]):Promise<void> {
    const blocks=new Map<number,string>();
    for(const item of items) {
      if(item.inclusion==="reorged") continue;
      let canonical=blocks.get(item.block_number);
      if(!canonical) {canonical=hash((await this.rpc.block(hexQuantity(item.block_number))).hash);blocks.set(item.block_number,canonical);}
      if(canonical!==item.block_hash) await this.journal.invalidateActivity(this.rpc.network.chain_id,account,item.id,item.block_hash,canonical);
    }
  }
  private async observe(account:string,txHash:string):Promise<Observation> {
    if(!this.core.validateActivity) throw new Error("Reload to complete the activity-validator update.");
    const [transaction,receipt]=await Promise.all([
      this.rpc.read("eth_getTransactionByHash",[txHash]),this.rpc.read("eth_getTransactionReceipt",[txHash]),
    ]);
    if(transaction===null && receipt===null) return {status:"not_found",items:[],warnings:[]};
    if(!transaction || typeof transaction!=="object" || Array.isArray(transaction)) throw new Error("The provider returned inconsistent transaction evidence.");
    const tx=transaction as Record<string,unknown>;
    if(hash(tx.hash)!==txHash) throw new Error("The provider returned another transaction.");
    if(receipt===null || tx.blockHash===null || tx.blockNumber===null) return {status:"pending",items:[],warnings:[]};
    if(!receipt || typeof receipt!=="object" || Array.isArray(receipt)) throw new Error("Invalid activity receipt.");
    const observed=receipt as Record<string,unknown>;
    if(hash(observed.transactionHash)!==txHash) throw new Error("The provider returned another transaction receipt.");
    const height=safeNumber(quantity(observed.blockNumber));
    const block=await this.rpc.block(hexQuantity(height));
    if(hash(observed.blockHash)!==hash(block.hash)) throw new Error("This receipt is not currently in the canonical chain. Earlier observations are preserved.");
    let finalized=0;const warnings:string[]=[];
    try {finalized=safeNumber(quantity((await this.rpc.block("finalized")).number));}
    catch {warnings.push("Finality could not be read. New activity is shown as included, without a finality claim.");}
    const proof:ActivityProof={transaction,receipt,canonical_block:block,finalized_height:finalized};
    const validated=await this.core.validateActivity(this.rpc.network.chain_id,account,proof);
    await this.rpc.assertCanonical(block);
    const prior=await this.journal.transactionActivity(this.rpc.network.chain_id,account,txHash);
    await this.correctPrior(account,prior);
    if(!validated.activities.length && prior.some(item=>item.inclusion!=="reorged" && item.block_hash===hash(block.hash))) throw new Error("This receipt contradicts a recorded transfer at the same canonical block. The prior evidence is preserved.");
    if(quantity(observed.status)===0n) return {status:"failed",proof,items:[],warnings};
    const persisted=await this.journal.recordActivity(this.rpc.network.chain_id,account,proof);
    if(persisted.some(item=>item.inclusion==="reorged")) warnings.push("A previously finalized inclusion now has conflicting chain evidence. Its recorded incident is preserved and requires investigation.");
    return {status:"observed",proof,items:persisted,warnings};
  }
  async lookup(account:string,transactionHash:string):Promise<ActivityResult> {
    address(account);const txHash=hash(transactionHash);
    let saved=await this.journal.listActivity(this.rpc.network.chain_id,account);
    try {
      await this.rpc.verifyChain();
      await this.correctPrior(account,saved.filter(item=>item.transaction_hash===txHash));
      const observed=await this.observe(account,txHash);
      saved=await this.journal.listActivity(this.rpc.network.chain_id,account);
      let status:ActivityResult["lookup_status"];let message:string|null=null;
      if(observed.status==="not_found") {status="not_found";message=`This provider has not found that hash on ${this.rpc.network.network}. Check the selected network; this does not prove that a deposit failed.`;}
      else if(observed.status==="pending") {status="pending";message="The transaction is still pending. A receipt is required before a received amount can be confirmed.";}
      else if(observed.status==="failed") {status="failed";message="The transaction reverted on this network. Its receipt confirms no successful transfer.";}
      else if(observed.items.some(item=>item.inclusion==="reorged")) {status="unavailable";message="Conflicting inclusion evidence is recorded for this transaction. It is not being presented as a newly confirmed deposit.";}
      else if(!observed.items.length) {status="unrelated";message="No supported direct ETH or native USDC/EURC Transfer event involving this address was found in this receipt. Internal ETH transfers are not indexed; the on-chain balance remains authoritative.";}
      else status=observed.items.some(item=>item.direction==="received")?"received":observed.items[0].direction;
      return {...this.result(saved,"lookup",observed.warnings,message),lookup_status:status};
    } catch(error) {
      saved=await this.journal.listActivity(this.rpc.network.chain_id,account);
      return {...this.result(saved,"unavailable",[],errorText(error)),lookup_status:"unavailable"};
    }
  }
  async refresh(account:string):Promise<ActivityResult> {
    address(account);
    let saved=await this.journal.listActivity(this.rpc.network.chain_id,account);
    const warnings:string[]=[];let from:number|null=null;let to:number|null=null;
    try {
      await this.rpc.verifyChain();
      const latest=await this.rpc.block("latest",true);to=safeNumber(quantity(latest.number));
      const cursor=await this.journal.activityCursor(this.rpc.network.chain_id,account);
      const earliest=Math.max(0,to-MAX_BLOCKS+1);
      from=cursor===null?earliest:Math.max(earliest,Math.min(cursor-64,to));
      from=Math.max(0,from);
      if(cursor!==null && cursor<earliest) warnings.push("Only the most recent 8,000 blocks are scanned in this refresh. Use a transaction hash for older deposits.");
      await this.correctPrior(account,saved.slice(0,MAX_CANDIDATES));
      const candidates=new Set<string>();let indexedHashes:string[]=[];
      try {
        const indexed=await this.rpc.activityCandidates(account);
        indexedHashes=indexed.hashes.slice(0,MAX_CANDIDATES).map(value=>hash(value));
        if(indexed.partial || indexed.notice) warnings.push(indexed.notice ?? "The history index supplies a limited recent page, not a complete account statement.");
      } catch {warnings.push("The history index is unavailable. Recent supported-token logs are scanned; older or internal ETH deposits may require a transaction hash.");}
      const participant=`0x${account.slice(2).toLowerCase().padStart(64,"0")}`;
      const contracts=this.rpc.network.assets.filter(asset=>asset.contract).map(asset=>asset.contract!);
      let scanned=true;
      // Newest ranges first, at most four windows and two participant queries.
      for(let end=to;end>=from;end-=RANGE) {
        const start=Math.max(from,end-RANGE+1);
        const results=await Promise.allSettled([
          this.rpc.read("eth_getLogs",[{address:contracts,fromBlock:hexQuantity(start),toBlock:hexQuantity(end),topics:[TRANSFER,null,participant]}]),
          this.rpc.read("eth_getLogs",[{address:contracts,fromBlock:hexQuantity(start),toBlock:hexQuantity(end),topics:[TRANSFER,participant]}]),
        ]);
        for(const result of results) {
          if(result.status!=="fulfilled" || !Array.isArray(result.value) || result.value.length>2048) {scanned=false;continue;}
          for(const log of [...result.value].reverse()) {
            if(log && typeof log==="object" && log.removed!==true) candidates.add(hash(log.transactionHash));
          }
        }
      }
      if(!scanned) warnings.push("Some recent log ranges could not be read. Missing activity is not evidence that a deposit failed.");
      // Native direct ETH has no Transfer log. Bounded block discovery is a
      // fallback only; it never treats balance changes as transaction amounts.
      if(candidates.size<MAX_CANDIDATES) {
        for(let height=to;height>=Math.max(from,to-7);height--) {
          try {
            const block=await this.rpc.block(hexQuantity(height),false,true) as unknown as {transactions?:unknown[]};
            if(!Array.isArray(block.transactions)) continue;
            for(const candidate of block.transactions) {
              if(!candidate || typeof candidate!=="object") continue;
              const tx=candidate as Record<string,unknown>;
              if(quantity(tx.value)>0n && ((typeof tx.to==="string"&&sameAddress(tx.to,account)) || (typeof tx.from==="string"&&sameAddress(tx.from,account)))) candidates.add(hash(tx.hash));
            }
          } catch {warnings.push("Recent native ETH block discovery is incomplete. A pasted transaction hash can verify a direct deposit.");break;}
        }
      }
      // Recent receipt-backed logs take priority over optional index hints.
      // A stale index page must not crowd a fresh deposit out of this batch.
      indexedHashes.forEach(value=>candidates.add(value));
      if(candidates.size>MAX_CANDIDATES) {warnings.push(`Only ${MAX_CANDIDATES} candidate transactions are verified per refresh.`);scanned=false;}
      let failed=0;
      const hashes=[...candidates].slice(0,MAX_CANDIDATES);
      // Two concurrent verification paths, each bounded and strictly read-only.
      for(let offset=0;offset<hashes.length;offset+=2) {
        const observations=await Promise.allSettled(hashes.slice(offset,offset+2).map(value=>this.observe(account,value)));
        for(const observed of observations) {
          if(observed.status==="rejected") {failed++;continue;}
          if(observed.value.status==="pending" || observed.value.status==="not_found") failed++;
          warnings.push(...observed.value.warnings);
        }
      }
      if(failed) warnings.push(`${failed} discovered transaction${failed===1?"":"s"} could not yet be verified; existing activity is preserved.`);
      await this.rpc.assertCanonical(latest);
      if(scanned && !failed) await this.journal.recordActivityCursor(this.rpc.network.chain_id,account,to);
      saved=await this.journal.listActivity(this.rpc.network.chain_id,account);
      return this.result(saved,warnings.length?"partial":"recent",[...new Set(warnings)],null,from,to);
    } catch(error) {
      saved=await this.journal.listActivity(this.rpc.network.chain_id,account);
      return this.result(saved,"unavailable",[...new Set(warnings)],errorText(error),from,to);
    }
  }
}
