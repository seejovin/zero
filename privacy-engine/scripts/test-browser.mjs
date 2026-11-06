import {chromium} from 'playwright-core';
import {createServer} from 'node:http';
import {readFile,writeFile,mkdir,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {resolve} from 'node:path';
import {localKeys,feeRecipientCommitment,TOKEN} from '../src/protocol.mjs';
import {CANDIDATE} from '../src/candidate-calldata.mjs';
import assert from 'node:assert/strict';
const root=resolve(new URL('..',import.meta.url).pathname),out=resolve(root,'../build/private-engine-browser-test');await mkdir(out,{recursive:true});
const fee=await localKeys('21'.repeat(32),'22'.repeat(32));const hash='0x'+'11'.repeat(32);
const domain={protocol:CANDIDATE.protocol,chain_id:42161,pool_contract:CANDIDATE.pool,token_contract:TOKEN,token_decimals:6,verifier_digest:hash};
const fixture={schema_version:1,approved:false,domain,fee_recipient_address:fee.address,deployment_block:0,circuits:{}};
await writeFile(resolve(out,'fixture-manifest.json'),JSON.stringify(fixture));
const build=spawnSync(process.execPath,[resolve(root,'scripts/build-browser.mjs'),out,resolve(out,'fixture-manifest.json')],{cwd:root,encoding:'utf8'});if(build.status!==0)throw Error(build.stderr);
const server=createServer(async(req,res)=>{const name=req.url==='/'?'index.html':req.url?.slice(1);if(!['index.html','engine.js'].includes(name)){res.writeHead(404);res.end();return}try{const b=await readFile(resolve(out,name));res.writeHead(200,{'Content-Type':name.endsWith('.js')?'application/javascript':'text/html','Cache-Control':'no-store'});res.end(b)}catch{res.writeHead(500);res.end()}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const url='http://127.0.0.1:'+server.address().port;
let browser;const errors=[],external=[];
try{
 browser=await chromium.launch({executablePath:'/usr/bin/chromium',headless:true,args:['--no-sandbox','--disable-dev-shm-usage']});const page=await browser.newPage();page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>{if(!r.url().startsWith(url))external.push(r.url())});
 await page.goto(url,{waitUntil:'load'});
 const result=await page.evaluate(async({domain,feeAddress,feeCommitment})=>{
  const hash=x=>'0x'+x.repeat(64).slice(0,64),channel=new MessageChannel(),session=hash('7'),messages=[],waiters=new Map();
  let readyResolve;const ready=new Promise(r=>readyResolve=r);channel.port1.onmessage=e=>{const m=JSON.parse(e.data);messages.push(m.kind);
   if(m.kind==='ready')readyResolve(m);else if(m.kind==='port_request'){
    if(m.port!=='storage_read')throw Error('unexpectedport');channel.port1.postMessage(JSON.stringify({schema_version:1,kind:'port_result',session_id:session,request_id:m.request_id,port_request_id:m.port_request_id,result:{exists:false,base64:null,offset:0,total_bytes:0,sha256:null}}));
   }else if(m.kind==='result'||m.kind==='error'){const p=waiters.get(m.request_id);waiters.delete(m.request_id);p(m)}
  };
  window.postMessage(JSON.stringify({schema_version:1,kind:'connect',session_id:session}),location.origin,[channel.port2]);
  await Promise.race([ready,new Promise((_,reject)=>setTimeout(()=>reject(Error('engine ready timeout')),15000))]);
  const call=(requestId,command,payload)=>new Promise(resolve=>{waiters.set(requestId,resolve);channel.port1.postMessage(JSON.stringify({schema_version:1,kind:'request',session_id:session,request_id:requestId,command,payload}))});
  const initialized=await call(hash('8'),'initialize',{wallet_identity:hash('9'),spending_private_key_hex:'11'.repeat(32),viewing_private_key_hex:'12'.repeat(32),encrypted_state_key_hex:'13'.repeat(32),domain,fee_recipient_address:feeAddress,fee_recipient_commitment:feeCommitment,creation_block:0});
  const balance=await call(hash('a'),'balance',{chain_id:42161,token_contract:domain.token_contract});
  let fetchBlocked=false;try{await fetch('https://example.com/should-never-leave')}catch{fetchBlocked=true}
  const closed=await call(hash('b'),'close',{});channel.port1.close();return {initialized,balance,closed,fetchBlocked,messages};
 },{domain,feeAddress:fee.address,feeCommitment:feeRecipientCommitment(fee.address)});
 assert.equal(result.initialized.kind,'result',JSON.stringify(result));assert.equal(result.initialized.result.private_address.length,127);assert.equal(result.balance.result.units,'0');assert.equal(result.balance.result.scan_complete,false);assert.equal(result.fetchBlocked,true);assert.equal(result.closed.result.closed,true);assert.deepEqual(errors,[]);
 // Playwright emits a request event even when CSP blocks it; verify no successful
 // external response and the fetch promise rejects rather than disabling CSP.
 assert.ok(external.every(x=>x==='https://example.com/should-never-leave'));
 const report={schema_version:1,test:'actual Chromium offline engine + strict CSP + transferred port',passed:true,browser:await browser.version(),checks:['bundle executes with no unsafe-eval','SDK derives 127-character private address','native port initialization handshake','balance remains explicitly unscanned','CSP blocks outbound fetch','session closes'],android_webview_tested:false,mainnet_funds_used:false};
 await writeFile(resolve(out,'browser-verification.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
}finally{await browser?.close();await new Promise(r=>server.close(r))}
