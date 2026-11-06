/** The SDK's narrow Node-crypto compatibility surface. Cryptographic primitives
 * are supplied by pinned libraries and WebCrypto; no RSA/ECDSA/legacy key APIs. */
import {Buffer} from 'buffer';
import {sha256,sha512} from '@noble/hashes/sha2';
import aes from 'browserify-aes/browser.js';

export function randomFillSync(value){
 if(!(value instanceof Uint8Array)||!globalThis.crypto?.getRandomValues)throw Error('SECURE_RANDOM_UNAVAILABLE');
 for(let offset=0;offset<value.length;offset+=65536)globalThis.crypto.getRandomValues(value.subarray(offset,offset+65536));
 return value;
}
export function randomBytes(length){
 if(!Number.isSafeInteger(length)||length<0||length>1048576)throw Error('INVALID_RANDOM_LENGTH');
 return randomFillSync(Buffer.alloc(length));
}
export function createHash(name){
 const implementation={sha256,sha512}[name];if(!implementation)throw Error('UNSUPPORTED_HASH');
 const state=implementation.create();let finished=false;
 return {update(value,encoding){if(finished)throw Error('HASH_FINALIZED');state.update(typeof value==='string'?Buffer.from(value,encoding):value);return this},
  digest(encoding){if(finished)throw Error('HASH_FINALIZED');finished=true;const value=Buffer.from(state.digest());state.destroy();return encoding?value.toString(encoding):value}};
}
export const createCipheriv=aes.createCipheriv;
export const createDecipheriv=aes.createDecipheriv;
export default {randomFillSync,randomBytes,createHash,createCipheriv,createDecipheriv};
