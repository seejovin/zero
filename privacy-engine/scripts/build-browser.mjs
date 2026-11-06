import {build} from 'esbuild';
import {readFile,mkdir,writeFile,copyFile,stat} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {resolve,join,dirname} from 'node:path';
import {thirdPartyNotices} from './third-party-notices.mjs';
const dir=resolve(new URL('..',import.meta.url).pathname),out=resolve(process.argv[2]??join(dir,'dist'));
const supplied=process.argv[3];
const manifest=supplied?JSON.parse(await readFile(resolve(supplied),'utf8')):{schema_version:1,approved:false,domain:null,fee_recipient_address:null,deployment_block:0,circuits:{}};
if(manifest.schema_version!==1||typeof manifest.approved!=='boolean'||typeof manifest.circuits!=='object')throw Error('Invalid protocol manifest');
const artifactRoot=supplied?dirname(resolve(supplied)):null,pins=JSON.parse(await readFile(join(dir,'config/upstream-artifact-v2-hashes.json'),'utf8'));
await mkdir(out,{recursive:true});
const files=[];const hash=b=>createHash('sha256').update(b).digest('hex');
for(const [shape,artifacts]of Object.entries(manifest.circuits)){
 if(!/^(0[1-9]|1[01])x03$/.test(shape)||!pins[shape])throw Error('Unreviewed circuit shape');
 for(const kind of ['wasm','zkey','vkey']){const a=artifacts[kind];if(!a||a.path!==`artifacts/${shape}.${kind==='vkey'?'vkey.json':kind}`||!/^[0-9a-f]{64}$/.test(a.sha256))throw Error('Invalid circuit manifest');
  const source=join(artifactRoot,a.path),info=await stat(source);if(!info.isFile()||info.size<1||info.size>256*1024*1024)throw Error('Artifact exceeds Android asset limit');
  const b=await readFile(source);if(b.length!==a.bytes||hash(b)!==a.sha256||(kind!=='vkey'&&pins[shape][kind]!==a.sha256))throw Error('Artifact does not match pinned upstream circuit');
  await mkdir(join(out,'artifacts'),{recursive:true});await copyFile(join(artifactRoot,a.path),join(out,a.path));files.push({path:a.path,mime:kind==='wasm'?'application/wasm':kind==='vkey'?'application/json':'application/octet-stream',sha256:a.sha256,bytes:b.length});
 }
}
const disabledWasm={name:'offline-sdk',setup(b){b.onResolve({filter:/^@wallet\/release-manifest$/},()=>({path:'manifest',namespace:'pinned'}));b.onLoad({filter:/.*/,namespace:'pinned'},()=>({contents:JSON.stringify(manifest),loader:'json'}));
 b.onResolve({filter:/^micro-ftch$/},()=>({path:'blocked-network',namespace:'blocked-network'}));b.onLoad({filter:/.*/,namespace:'blocked-network'},()=>({contents:'module.exports = async function(){throw new Error("SDK_NETWORK_DISABLED")};',loader:'js'}));
 b.onResolve({filter:/^@railgun-community\/(poseidon-hash-wasm|curve25519-scalarmult-wasm)$/},()=>({path:'disabled',namespace:'offline-wasm'}));b.onLoad({filter:/.*/,namespace:'offline-wasm'},()=>({contents:'module.exports = {};',loader:'js'}));}};
const result=await build({entryPoints:[join(dir,'src/browser-entry.mjs')],outfile:join(out,'engine.js'),bundle:true,platform:'browser',format:'iife',target:'chrome100',minify:true,metafile:true,sourcemap:false,plugins:[disabledWasm],inject:[join(dir,'src/browser-globals.mjs')],define:{'process.env.NODE_ENV':'"production"','globalThis.Worker':'undefined'},alias:{crypto:join(dir,'src/browser-crypto.mjs'),stream:'stream-browserify',events:'events',buffer:'buffer',assert:'assert/'},logLevel:'warning'});
await copyFile(join(dir,'public/index.html'),join(out,'index.html'));
const notices=await thirdPartyNotices(Object.keys(result.metafile.inputs),process.cwd());
await writeFile(join(out,'third-party-notices.txt'),notices.text);
await writeFile(join(out,'third-party-inventory.json'),JSON.stringify(notices.inventory,null,2)+'\n');
for(const path of ['index.html','engine.js','third-party-notices.txt']){const b=await readFile(join(out,path));files.push({path,mime:path.endsWith('.js')?'application/javascript':path.endsWith('.txt')?'text/plain':'text/html',sha256:hash(b),bytes:b.length})}
files.sort((a,b)=>a.path.localeCompare(b.path));
if(files.reduce((sum,f)=>sum+f.bytes,0)>512*1024*1024)throw Error('Android private asset package exceeds limit');
const assets={schema_version:1,sdk_version:'local-wallet-engine-1',engine_version:'9.9.1',entry:'index.html',files};
const bytes=Buffer.from(JSON.stringify(assets));await writeFile(join(out,'manifest.json'),bytes);
await writeFile(join(out,'bundle-inputs.json'),JSON.stringify(Object.keys(result.metafile.inputs).sort(),null,2)+'\n');
console.log(JSON.stringify({output:out,manifest_sha256:hash(bytes),assets:files.length,javascript_bytes:files.find(f=>f.path==='engine.js').bytes,circuit_shapes:Object.keys(manifest.circuits),approved:manifest.approved}));
