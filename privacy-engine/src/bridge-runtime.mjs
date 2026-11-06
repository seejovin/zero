/** A single transferred MessagePort, no WebView JavascriptInterface or eval. */
import {LocalPrivateEngine} from './local-engine.mjs';
const HASH=/^0x[0-9a-f]{64}$/,MAX=2097152;
const codes=new Set(['INVALID_INPUT','SYNC_INCOMPLETE','INSUFFICIENT_FUNDS','QUOTE_EXPIRED','CONSOLIDATION_REQUIRED','STORAGE','BACKUP_REQUIRED','PROOF_INVALID','PROTOCOL_MISMATCH','CANCELLED','UNAVAILABLE','TRANSPORT_UNAVAILABLE','RECOVERY_WAITING_FOR_FINALITY','RECOVERY_CUTOFF_REORG','RECOVERY_CAPACITY_EXCEEDED','RECOVERY_FEE_EXCEEDS_BALANCE','RECOVERY_NEEDS_RECONCILIATION','RECOVERY_NOTE_MISMATCH','RECOVERY_REVIEW_IN_PROGRESS','RECOVERY_SELF_TRANSFER_ONLY']);
const commands=new Set(['initialize','sync','balance','prepare','prove','verify','observe','export_state','import_state','restore_reservations','release_cancelled_reservations','recovery_cutoff','recovery_scan','recovery_prepare','cancel','close']);
const phases={initialize:'initializing',sync:'syncing',prepare:'preparing',prove:'proving',verify:'verifying',observe:'reconciling',export_state:'exporting',import_state:'importing',recovery_cutoff:'reconciling',recovery_scan:'syncing',recovery_prepare:'preparing'};
const equalFields=(m,fields)=>Object.keys(m).sort().join(',')===fields.sort().join(',');
function parse(text){if(typeof text!=='string'||text.length>MAX)throw Error('INVALID_INPUT');const m=JSON.parse(text);if(!m||Array.isArray(m)||typeof m!=='object'||m.schema_version!==1||!HASH.test(m.session_id))throw Error('INVALID_INPUT');return m}
const randomId=()=>{const b=crypto.getRandomValues(new Uint8Array(32));return '0x'+Array.from(b,x=>x.toString(16).padStart(2,'0')).join('')};
export function connectRuntime(port,session,manifest){
 const pending=new Map(),seen=new Set();let active=null,closed=false;
 const send=(kind,data={})=>{if(closed)return;const value=JSON.stringify({schema_version:1,kind,session_id:session,...data});if(value.length>MAX)throw Error('INVALID_INPUT');port.postMessage(value)};
 const localPort=(name,payload)=>new Promise((resolve,reject)=>{if(!active||closed||pending.size>16)return reject(Error('UNAVAILABLE'));const key=randomId();const timer=setTimeout(()=>{pending.delete(key);reject(Error('UNAVAILABLE'))},60000);pending.set(key,{request:active,resolve,reject,timer});send('port_request',{request_id:active,port_request_id:key,port:name,payload})});
 const engine=new LocalPrivateEngine(manifest,localPort);
 const destroy=()=>{engine.close();closed=true;for(const p of pending.values()){clearTimeout(p.timer);p.reject(Error('CANCELLED'))}pending.clear();port.close()};
 port.onmessage=async event=>{
  let m;try{m=parse(event.data);if(m.session_id!==session)throw Error('INVALID_INPUT')}catch{destroy();return}
  if(m.kind==='port_result'||m.kind==='port_error'){
   if(!equalFields(m,['schema_version','kind','session_id','request_id','port_request_id',m.kind==='port_result'?'result':'code'])){destroy();return}
   const p=pending.get(m.port_request_id);if(!p||p.request!==m.request_id){destroy();return}pending.delete(m.port_request_id);clearTimeout(p.timer);m.kind==='port_result'?p.resolve(m.result):p.reject(Error('UNAVAILABLE'));return;
  }
  if(m.kind!=='request'||!equalFields(m,['schema_version','kind','session_id','request_id','command','payload'])||!HASH.test(m.request_id)||!commands.has(m.command)||seen.has(m.request_id)||seen.size>100000){destroy();return}
  if(active){send('error',{request_id:m.request_id,code:'UNAVAILABLE'});return}seen.add(m.request_id);active=m.request_id;
  try{
   const progress=percent=>send('progress',{request_id:m.request_id,phase:phases[m.command],percent:Math.max(0,Math.min(100,Math.floor(percent)))});
   if(phases[m.command])progress(0);
   let result;
   if(m.command==='export_state')result=await engine.exportState();else if(m.command==='import_state')result=await engine.importState();else result=await engine[m.command](m.payload,progress);
   send('result',{request_id:m.request_id,result});if(m.command==='close')destroy();
  }catch(e){send('error',{request_id:m.request_id,code:codes.has(e?.message)?e.message:'LOCAL_ENGINE'})}finally{active=null}
 };
 port.start?.();send('ready',{protocol:'railgun-v2-candidate',sdk_version:'local-wallet-engine-1',engine_version:'9.9.1'});return {destroy,engine};
}
export function installRuntime(manifest){let connected=false;globalThis.addEventListener('message',event=>{
 if(connected||event.ports?.length!==1)return;
 let m;try{m=parse(event.data);if(m.kind!=='connect'||!equalFields(m,['schema_version','kind','session_id']))return}catch{return}
 // Only native WebView code can supply this port. No frames/navigation/network
 // exist in this synthetic origin; subsequent messages require session binding.
 connected=true;connectRuntime(event.ports[0],m.session_id,manifest);
 });}
