#!/usr/bin/env node
// Fixed subprocess API. No networking, telemetry, user context or shell commands.
import {readFile} from 'node:fs/promises';
import {resolve,dirname,isAbsolute,relative} from 'node:path';
import {verifyProviderEnvelope} from '../src/provider-verifier.mjs';
process.stdout.on('error',()=>process.exit(1));
const failure=()=>{process.stdout.write('{"error":"verification_failed"}\n',()=>process.exit(1))};
try{
 if(process.argv.length!==4||process.argv[2]!=='--config'||!isAbsolute(process.argv[3]))throw 0;
 const file=await readFile(process.argv[3]);if(file.length>131072)throw 0;
 const config=JSON.parse(file.toString('utf8'));file.fill(0);
 const parts=[];let total=0;for await(const chunk of process.stdin){total+=chunk.length;if(total>524288)throw 0;parts.push(chunk)}
 const request=JSON.parse(Buffer.concat(parts).toString('utf8'));for(const p of parts)p.fill(0);
 if(Object.keys(request).sort().join(',')!=='envelope,schema_version'||request.schema_version!==1)throw 0;
 // Paths are fixed operator configuration and confined to one artifact directory.
 const root=resolve(dirname(process.argv[3]),config.artifact_directory);
 const result=await verifyProviderEnvelope(request.envelope,config,async name=>{if(typeof name!=='string'||isAbsolute(name)||relative(root,resolve(root,name)).startsWith('..'))throw 0;const bytes=await readFile(resolve(root,name));if(bytes.length>2000000)throw 0;return bytes});
 config.fee_viewing_private_key_hex='';
 // snarkjs may keep public curve workers alive. This is a one-request process;
 // terminate only after the complete bounded response has flushed to the pipe.
 process.stdout.write(JSON.stringify(result)+'\n',()=>process.exit(0));
}catch{failure()}
