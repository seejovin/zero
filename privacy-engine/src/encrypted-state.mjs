import {sha256,getBytes} from 'ethers';
const KEY='private-engine-state-v1',MAX=64*1024*1024,CHUNK=262144;
const base64=bytes=>{let out='';for(let i=0;i<bytes.length;i+=8192)out+=String.fromCharCode(...bytes.subarray(i,i+8192));return btoa(out)};
const unbase64=s=>Uint8Array.from(atob(s),c=>c.charCodeAt(0));
export class EncryptedState{
 constructor(keyHex,identity,port){this.keyHex=keyHex;this.identity=identity;this.port=port}
 async key(){if(!this.keyHex||!/^[0-9a-f]{64}$/.test(this.keyHex))throw new Error('STORAGE');return crypto.subtle.importKey('raw',getBytes('0x'+this.keyHex),'AES-GCM',false,['encrypt','decrypt'])}
 aad(){return new TextEncoder().encode('local-wallet/private-engine-state/v1\0'+this.identity)}
 async save(snapshot){
  const plain=new TextEncoder().encode(JSON.stringify(snapshot));if(plain.length>MAX-32)throw new Error('STORAGE');
  const iv=crypto.getRandomValues(new Uint8Array(12));let encrypted;
  try{encrypted=new Uint8Array(await crypto.subtle.encrypt({name:'AES-GCM',iv,additionalData:this.aad()},await this.key(),plain))}finally{plain.fill(0)}
  const bytes=new Uint8Array(1+12+encrypted.length);bytes[0]=1;bytes.set(iv,1);bytes.set(encrypted,13);encrypted.fill(0);
  const hash=sha256(bytes).slice(2);let upload;
  try{upload=await this.port('storage_write',{action:'begin',key:KEY,total_bytes:bytes.length,sha256:hash});
   for(let offset=0;offset<bytes.length;offset+=CHUNK){const end=Math.min(offset+CHUNK,bytes.length);const r=await this.port('storage_write',{action:'write',key:KEY,upload_id:upload.upload_id,offset,base64:base64(bytes.subarray(offset,end))});if(r.offset!==end)throw new Error('STORAGE')}
   const r=await this.port('storage_write',{action:'commit',key:KEY,upload_id:upload.upload_id});if(r.committed!==true||r.total_bytes!==bytes.length||r.sha256!==hash)throw new Error('STORAGE');
  }catch(e){if(upload)try{await this.port('storage_write',{action:'abort',key:KEY,upload_id:upload.upload_id})}catch{}throw new Error('STORAGE')}finally{bytes.fill(0)}
 }
 async load(){
  let first=await this.port('storage_read',{key:KEY,offset:0,length:CHUNK,sha256:null});if(first.exists===false)return null;
  if(!Number.isSafeInteger(first.total_bytes)||first.total_bytes<29||first.total_bytes>MAX||!/^[0-9a-f]{64}$/.test(first.sha256))throw new Error('STORAGE');
  const bytes=new Uint8Array(first.total_bytes);let offset=0,r=first;
  try{while(offset<bytes.length){if(r.offset!==offset||r.total_bytes!==bytes.length||r.sha256!==first.sha256)throw new Error('STORAGE');const chunk=unbase64(r.base64);if(!chunk.length||chunk.length>CHUNK||offset+chunk.length>bytes.length)throw new Error('STORAGE');bytes.set(chunk,offset);offset+=chunk.length;chunk.fill(0);if(offset<bytes.length)r=await this.port('storage_read',{key:KEY,offset,length:Math.min(CHUNK,bytes.length-offset),sha256:first.sha256})}
   if(sha256(bytes).slice(2)!==first.sha256||bytes[0]!==1)throw new Error('STORAGE');
   const plain=new Uint8Array(await crypto.subtle.decrypt({name:'AES-GCM',iv:bytes.slice(1,13),additionalData:this.aad()},await this.key(),bytes.slice(13)));
   try{return JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(plain))}finally{plain.fill(0)}
  }catch{throw new Error('STORAGE')}finally{bytes.fill(0)}
 }
 destroy(){this.keyHex=''}
}
export {base64,unbase64};
