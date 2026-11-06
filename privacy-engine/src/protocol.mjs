/** Narrow pinned protocol primitives. No engine start, RPC, subgraph or POI service. */
import keys from '../node_modules/@railgun-community/engine/dist/utils/keys-utils.js';
import bech32 from '../node_modules/@railgun-community/engine/dist/key-derivation/bech32.js';
import noteModule from '../node_modules/@railgun-community/engine/dist/note/transact-note.js';
import shieldModule from '../node_modules/@railgun-community/engine/dist/note/shield-note.js';
import noteUtil from '../node_modules/@railgun-community/engine/dist/note/note-util.js';
import transactionModule from '../node_modules/@railgun-community/engine/dist/transaction/transaction.js';
import proverModule from '../node_modules/@railgun-community/engine/dist/prover/prover.js';
import formatted from '../node_modules/@railgun-community/engine/dist/models/formatted-types.js';
import poi from '../node_modules/@railgun-community/engine/dist/models/poi-types.js';
import circom from '@railgun-community/circomlibjs';
import walletInfoModule from '../node_modules/@railgun-community/engine/dist/wallet/wallet-info.js';
import { keccak256, toUtf8Bytes, getBytes, hexlify, ZeroAddress, ZeroHash } from 'ethers';
export const FIELD=21888242871839275222246405745257275088548364400416034343698204186575808495617n;
// One common encrypted protocol tag; never a wallet, device or customer ID.
walletInfoModule.default.setWalletSource('localwallet');
export const CHAIN=Object.freeze({type:0,id:42161});
export const TOKEN='0xaf88d065e77c8cc2239327c5edb3a432268e5831';
export const VERSION=poi.TXIDVersion.V2_PoseidonMerkle;
export const { TransactNote }=noteModule;
export const { ShieldNote }=shieldModule;
export const { Transaction }=transactionModule;
export const { Prover }=proverModule;
export const { OutputType }=formatted;
export const { getTokenDataERC20, getTokenDataHash, getNoteHash }=noteUtil;
export const poseidon=values=>circom.poseidon(values);
export const hex32=n=>'0x'+BigInt(n).toString(16).padStart(64,'0');
export const bytesHex=bytes=>hexlify(bytes).slice(2);
export const parseUnits=value=>{if(typeof value!=='string'||!/^(0|[1-9][0-9]{0,36})$/.test(value)||BigInt(value)>=(1n<<120n))throw new Error('INVALID_AMOUNT');return BigInt(value)};
export function addressData(address){
 if(typeof address!=='string'||address.length!==127||address!==address.toLowerCase())throw new Error('INVALID_RECIPIENT');
 const d=bech32.decodeAddress(address);
 if(bech32.encodeAddress(d)!==address||d.masterPublicKey<=0n||d.masterPublicKey>=FIELD||d.viewingPublicKey.length!==32
   ||(d.chain&&(d.chain.type!==0||d.chain.id!==42161)))throw new Error('INVALID_RECIPIENT');
 return d;
}
export const feeRecipientCommitment=address=>{addressData(address);return keccak256(toUtf8Bytes('local-wallet/provider-fee-recipient/v1\0'+address))};
export async function localKeys(spendingHex,viewingHex){
 if(!/^[0-9a-f]{64}$/.test(spendingHex)||! /^[0-9a-f]{64}$/.test(viewingHex)||/^0+$/.test(spendingHex)||/^0+$/.test(viewingHex))throw new Error('INVALID_KEY');
 const spending=getBytes('0x'+spendingHex),viewing=getBytes('0x'+viewingHex);
 const spendingPublic=keys.getPublicSpendingKey(spending),viewingPublic=await keys.getPublicViewingKey(viewing);
 const nullifyingKey=poseidon([BigInt('0x'+viewingHex)]),masterPublicKey=poseidon([...spendingPublic,nullifyingKey]);
 const data={masterPublicKey,viewingPublicKey:viewingPublic};
 return {spending,viewing,spendingPublic,viewingPublic,nullifyingKey,data,address:bech32.encodeAddress(data),
   destroy(){spending.fill(0);viewing.fill(0);spendingPublic.fill(0n);viewingPublic.fill(0);this.nullifyingKey=0n;this.data.masterPublicKey=0n;}};
}
export const tokenGetter=Object.freeze({async getTokenDataFromHash(_version,_chain,hash){
 const h=BigInt('0x'+String(hash).replace(/^0x/,''));
 if(h!==BigInt(TOKEN))throw new Error('UNSUPPORTED_ASSET'); return getTokenDataERC20(TOKEN);
}});
export function isSupportedNote(note){return note.tokenData.tokenType===0&&BigInt(note.tokenData.tokenSubID)===0n&&BigInt(note.tokenData.tokenAddress)===BigInt(TOKEN)&&note.value>=0n&&note.value<(1n<<120n)}
export function makeTransfer(to,own,value,type=OutputType.Transfer){
 if(value<0n||value>=(1n<<120n))throw new Error('INVALID_AMOUNT');
 return TransactNote.createTransfer(addressData(to),own.data,value,getTokenDataERC20(TOKEN),false,type,undefined);
}
export async function decryptTransact(cipher,commitment,own){
 const shared=await keys.getSharedSymmetricKey(own.viewing,getBytes(cipher.blindedSenderViewingKey));if(!shared)return null;
 try{
 const first=cipher.ciphertext[0].replace(/^0x/,'');
 const note=await TransactNote.decrypt(VERSION,CHAIN,own.data,{iv:first.slice(0,32),tag:first.slice(32),data:cipher.ciphertext.slice(1).map(x=>x.replace(/^0x/,''))},shared,cipher.memo.replace(/^0x/,''),cipher.annotationData.replace(/^0x/,''),own.viewing,getBytes(cipher.blindedReceiverViewingKey),getBytes(cipher.blindedSenderViewingKey),false,false,tokenGetter,undefined,undefined);
 if(hex32(note.hash)!==hex32(commitment)||!isSupportedNote(note)||bech32.encodeAddress(note.receiverAddressData)!==own.address)return null;
 return note;
 }catch{return null}finally{shared.fill(0)}
}
export async function decryptShield(preimage,cipher,own){
 const shared=await keys.getSharedSymmetricKey(own.viewing,getBytes(cipher.shieldKey));if(!shared)return null;
 try{
 const token={tokenType:Number(preimage.token.tokenType),tokenAddress:preimage.token.tokenAddress.toLowerCase(),tokenSubID:hex32(preimage.token.tokenSubID)};
 if(token.tokenType!==0||token.tokenAddress!==TOKEN||BigInt(token.tokenSubID)!==0n)return null;
 const random=ShieldNote.decryptRandom(cipher.encryptedBundle,shared);
 const serialized={npk:preimage.npk.replace(/^0x/,''),random,tokenHash:getTokenDataHash(token),value:BigInt(preimage.value).toString(16).padStart(32,'0'),recipientAddress:own.address};
 const note=await TransactNote.deserialize(VERSION,CHAIN,serialized,own.viewing,tokenGetter);
 if(note.hash!==getNoteHash(preimage.npk,token,BigInt(preimage.value))||!isSupportedNote(note))return null;
 return note;
 }catch{return null}finally{shared.fill(0)}
}
export function walletFacade(own,forest){return {
 addressKeys:own.data,viewingKeyPair:{privateKey:own.viewing,pubkey:own.viewingPublic},
 getUTXOMerkletree(){return {async getRoot(tree){return forest.root(tree).slice(2)},async getMerkleProof(tree,index){return {elements:forest.proof(tree,index).map(x=>x.slice(2))}}}},
 async getSpendingKeyPair(){return {privateKey:own.spending,pubkey:own.spendingPublic}},
 getNullifyingKey(){return own.nullifyingKey},getViewingKeyPair(){return this.viewingKeyPair},
 sign(publicInputs){return keys.signEDDSA(own.spending,poseidon([publicInputs.merkleRoot,publicInputs.boundParamsHash,...publicInputs.nullifiers,...publicInputs.commitmentsOut]))},
};}
export const NO_ADAPT=Object.freeze({contract:ZeroAddress,parameters:ZeroHash});
