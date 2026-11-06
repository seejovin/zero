import {poseidon,FIELD,hex32} from './protocol.mjs';
import {keccak256,toUtf8Bytes} from 'ethers';
const DEPTH=16,CAPACITY=1<<DEPTH;
const zeros=[BigInt(keccak256(toUtf8Bytes('Railgun')))%FIELD];
for(let i=0;i<DEPTH;i++)zeros.push(poseidon([zeros[i],zeros[i]]));
/** Indexed append-only public pool tree. Hashes use the pinned upstream Poseidon.
 * Never accepts gaps/overwrites; complete historical data and contract root
 * verification are mandatory before any spend. */
export class MerkleForest{
 constructor(){this.trees=new Map()}
 tree(id){if(!Number.isSafeInteger(id)||id<0||id>65535)throw new Error('INVALID_TREE');if(!this.trees.has(id))this.trees.set(id,{count:0,levels:Array.from({length:DEPTH+1},()=>new Map())});return this.trees.get(id)}
 append(id,start,hashes){const t=this.tree(id);if(start!==t.count||!Array.isArray(hashes)||!hashes.length||start+hashes.length>CAPACITY)throw new Error('INCOMPLETE_TREE');
  for(const v of hashes){const h=BigInt(v);if(h<=0n||h>=FIELD)throw new Error('INVALID_COMMITMENT');let index=t.count++,value=h;t.levels[0].set(index,value);
   for(let level=0;level<DEPTH;level++){const sibling=t.levels[level].get(index^1)??zeros[level];value=poseidon(index&1?[sibling,value]:[value,sibling]);index>>=1;t.levels[level+1].set(index,value);}
  }
 }
 count(id){return this.tree(id).count}
 root(id){return hex32(this.tree(id).levels[DEPTH].get(0)??zeros[DEPTH])}
 proof(id,index){const t=this.tree(id);if(!Number.isSafeInteger(index)||index<0||index>=t.count)throw new Error('MISSING_NOTE');const out=[];for(let i=0;i<DEPTH;i++){out.push(hex32(t.levels[i].get(index^1)??zeros[i]));index>>=1}return out}
 leaf(id,index){const h=this.tree(id).levels[0].get(index);if(h===undefined)throw new Error('MISSING_NOTE');return hex32(h)}
 export(){return [...this.trees].map(([tree,t])=>({tree,leaves:[...t.levels[0].values()].map(hex32)}))}
 static import(snapshot){if(!Array.isArray(snapshot)||snapshot.length>512)throw new Error('INVALID_STATE');const f=new MerkleForest();for(const t of snapshot){if(f.trees.has(t.tree)||!Array.isArray(t.leaves)||t.leaves.length>CAPACITY)throw new Error('INVALID_STATE');if(t.leaves.length)f.append(t.tree,0,t.leaves)}return f}
 static checkProof(leaf,index,elements,root){if(elements.length!==DEPTH)return false;let h=BigInt(leaf);for(const e of elements){h=poseidon(index&1?[BigInt(e),h]:[h,BigInt(e)]);index>>=1}return index===0&&hex32(h)===root}
}
