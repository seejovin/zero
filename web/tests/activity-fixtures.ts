import type { ActivityProof, Asset, ChainId } from "../src/types";
import { blockHash, FROM, TO, networks } from "./fixtures";
export const ACTIVITY_HASH=blockHash("aa");
export const topic=(a:string)=>`0x${a.slice(2).toLowerCase().padStart(64,"0")}`;
export const TRANSFER="0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
export function activityProof(chain:ChainId=8453,asset:Asset="USDC",finalized=99):ActivityProof {
  const network=networks.find((n:any)=>n.chain_id===chain);
  const destination=asset==="ETH"?FROM:network.assets.find((a:any)=>a.symbol===asset).contract;
  const value=asset==="ETH"?"0x2386f26fc10000":"0x0";
  const transaction={hash:ACTIVITY_HASH,chainId:`0x${chain.toString(16)}`,blockHash:blockHash(),blockNumber:"0x64",from:TO,to:destination,value};
  const receipt={transactionHash:ACTIVITY_HASH,blockHash:blockHash(),blockNumber:"0x64",from:TO,to:destination,status:"0x1",logs:asset==="ETH"?[]:[{address:destination,topics:[TRANSFER,topic(TO),topic(FROM)],data:`0x${(10000n).toString(16).padStart(64,"0")}`,transactionHash:ACTIVITY_HASH,blockHash:blockHash(),blockNumber:"0x64",logIndex:"0x9",removed:false}]};
  return {transaction,receipt,canonical_block:{hash:blockHash(),number:"0x64",timestamp:`0x${Math.floor(Date.now()/1000).toString(16)}`,baseFeePerGas:"0x3b9aca00",transactions:[ACTIVITY_HASH]},finalized_height:finalized};
}
export function moveProof(proof:ActivityProof,nextHash=blockHash("22")):ActivityProof {
  const next=structuredClone(proof);(next.transaction as any).blockHash=nextHash;(next.receipt as any).blockHash=nextHash;(next.canonical_block as any).hash=nextHash;
  for(const log of (next.receipt as any).logs)log.blockHash=nextHash;
  return next;
}
