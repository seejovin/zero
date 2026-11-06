#!/usr/bin/env node
/** Authenticate downloaded/public circuit files against the pinned upstream
 * catalog and derive each verification key from its authenticated proving key.
 * This does not approve a deployment, change a release gate or create a setup. */
import {readFile,writeFile,mkdir,stat} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {brotliDecompressSync} from 'node:zlib';
import {zKey} from 'snarkjs';

const [inputArg,outputArg,shapeArg='01x03']=process.argv.slice(2);
if(!inputArg||!outputArg||process.argv.length>5)throw Error('Usage: qualify-artifacts.mjs INPUT_DIRECTORY OUTPUT_DIRECTORY [01x03,02x03,...]');
const input=resolve(inputArg),output=resolve(outputArg),shapes=shapeArg.split(',');
if(input===output||new Set(shapes).size!==shapes.length||!shapes.length||shapes.some(s=>!/^(0[1-9]|1[01])x03$/.test(s)))throw Error('Invalid circuit selection');
const catalog=JSON.parse(await readFile(new URL('../config/upstream-artifact-v2-hashes.json',import.meta.url),'utf8'));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(k=>[k,canonical(value[k])])):value;
const LIMIT=256*1024*1024;
async function readBounded(file){const s=await stat(file);if(!s.isFile()||s.size<1||s.size>LIMIT)throw Error('Artifact exceeds the package limit');return readFile(file)}
async function source(shape,kind){
 const plain=join(input,`${shape}.${kind}`);
 let bytes;try{bytes=await readBounded(plain)}catch(e){if(e.code!=='ENOENT')throw e;bytes=brotliDecompressSync(await readBounded(plain+'.br'),{maxOutputLength:LIMIT})}
 if(hash(bytes)!==catalog[shape]?.[kind])throw Error(`Upstream ${shape} ${kind} SHA-256 mismatch`);
 return bytes;
}
async function immutable(file,bytes){try{await writeFile(file,bytes,{flag:'wx',mode:0o644})}catch(e){if(e.code!=='EEXIST'||!Buffer.from(await readFile(file)).equals(Buffer.from(bytes)))throw Error('Existing artifact differs; choose a new output directory')}}
await mkdir(join(output,'artifacts'),{recursive:true});
const circuits={},checks=[];let total=0;
for(const shape of shapes){
 const wasm=await source(shape,'wasm'),zkey=await source(shape,'zkey');
 const key=await zKey.exportVerificationKey(new Uint8Array(zkey));
 if(key.protocol!=='groth16'||key.curve!=='bn128'||key.nPublic!==Number(shape.slice(0,2))+5||key.IC?.length!==key.nPublic+1)throw Error('Unexpected proving-key public-signal layout');
 let publishedCompared=false;
 try{const published=JSON.parse((await readBounded(join(input,shape+'.vkey.json'))).toString('utf8'));if(JSON.stringify(canonical(published))!==JSON.stringify(canonical(key)))throw Error('Published verification key does not match the pinned proving key');publishedCompared=true}catch(e){if(e.code!=='ENOENT')throw e}
 const vkey=Buffer.from(JSON.stringify(key));circuits[shape]={};
 for(const [kind,bytes] of Object.entries({wasm,zkey,vkey})){
  const path=`artifacts/${shape}.${kind==='vkey'?'vkey.json':kind}`;total+=bytes.length;if(total>500*1024*1024)throw Error('Circuit set leaves insufficient APK asset space');
  await immutable(join(output,path),bytes);circuits[shape][kind]={path,sha256:hash(bytes),bytes:bytes.length};
 }
 checks.push({shape,wasm_upstream_pin:true,zkey_upstream_pin:true,verification_key_derived:true,published_key_matches:publishedCompared,public_signals:key.nPublic});
 process.stderr.write(`Authenticated ${shape}; verification key derived from the pinned proving key.\n`);
}
const report={schema_version:1,sdk_engine_version:'9.9.1',catalog_source:'@railgun-community/wallet@11.3.0',
 ipfs_cid:'QmUsmnK4PFc7zDp2cmC4wBZxYLjNyRgWfs5GNcJJ2uLcpU',circuits,checks,
 deployed_verifier_correspondence_checked:false,independent_review:false,production_enabled:false};
await immutable(join(output,'qualified-circuits.json'),Buffer.from(JSON.stringify(report,null,2)+'\n'));
process.stdout.write(JSON.stringify({output,shapes,total_artifact_bytes:total,upstream_pins_verified:true,verification_keys_derived:true,production_enabled:false})+'\n');
// snarkjs key export may retain worker threads. All file writes above are awaited.
process.exit(0);
