import {Buffer} from 'buffer';
import process from 'process';
const global=globalThis;
// Refuse startup before any SDK RNG can reach an insecure compatibility path.
if(!globalThis.crypto?.getRandomValues||!globalThis.crypto?.subtle)throw Error('SECURE_RANDOM_UNAVAILABLE');
export {Buffer,process,global};
