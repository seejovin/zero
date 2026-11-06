import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { NetworkClient } from "../src/network";
import { RpcError, READ_PROVIDERS, resetRelayReadStateForTests } from "../src/transport";
import { networks, FROM, TO, blockHash, prepare, sign } from "./fixtures";
import { hexQuantity } from "../src/units";
import { relayRpc, resetRelayStateForTests } from "../../worker/relay";
const ethereum=networks.find((n:any)=>n.chain_id===1),base=networks.find((n:any)=>n.chain_id===8453);
const word=(value:number)=>`0x${BigInt(value).toString(16).padStart(64,"0")}`;
function response(body:any,result:unknown,error=false):Response {return new Response(JSON.stringify({jsonrpc:"2.0",id:body.id,...(error?{error:result}:{result})}));}
function unavailable(body:any,status=503):Response {return new Response(JSON.stringify({jsonrpc:"2.0",id:body.id,error:{code:-32098,message:"Unavailable"}}),{status});}
beforeEach(resetRelayReadStateForTests);

test("a relay 503 permits complete balance reads from a chain-checked fixed provider",async()=>{
  const calls:{url:string;body:any;credentials:RequestCredentials|undefined}[]=[];
  const block={hash:blockHash(),number:"0x64",timestamp:hexQuantity(Math.floor(Date.now()/1000)),baseFeePerGas:"0x3b9aca00"};
  const usdc=base.assets.find((asset:any)=>asset.symbol==="USDC").contract.toLowerCase();
  const fetcher=(async(url:unknown,init:RequestInit)=>{
    const body=JSON.parse(String(init.body));calls.push({url:String(url),body,credentials:init.credentials});
    if(String(url)==="/api/rpc") return body.method==="eth_chainId"?response(body,"0x2105"):unavailable(body);
    assert.equal(String(url),READ_PROVIDERS[8453][0]);
    if(body.method==="eth_chainId") return response(body,"0x2105");
    if(body.method==="eth_getBlockByNumber") return response(body,block);
    if(body.method==="eth_getBalance") return response(body,"0x1234");
    if(body.method==="eth_call") return response(body,word(body.params[0].to.toLowerCase()===usdc?10_000:2_500_000));
    throw new Error(`Unexpected read ${body.method}`);
  }) as typeof fetch;
  const rpc=new NetworkClient(base,fetcher,{relay:"/api/rpc"});
  const result=await rpc.balanceStatus(FROM);
  assert.equal(result.status,"fresh");
  assert.deepEqual(result.snapshot?.balances,{ETH:"4660",USDC:"10000",EURC:"2500000"});
  assert.equal(result.snapshot?.block_hash,block.hash);
  const direct=calls.filter(call=>call.url!=="/api/rpc");
  assert.equal(direct[0].body.method,"eth_chainId");
  assert.ok(direct.every(call=>call.credentials==="omit"));
  for(const call of direct.filter(call=>["eth_call","eth_getBalance"].includes(call.body.method))) assert.deepEqual(call.body.params[1],{blockHash:block.hash,requireCanonical:true});
  assert.equal(direct.at(-1)?.body.method,"eth_getBlockByNumber");
  assert.equal(direct.at(-1)?.body.params[0],block.number);
  assert.equal(rpc.connectionInfo().transport,"direct");
  assert.equal(rpc.connectionInfo().endpoint,READ_PROVIDERS[8453][0]);
  assert.ok(!calls.some(call=>call.body.method==="eth_sendRawTransaction"));
});
test("only explicit temporary HTTP statuses authorize relay read fallback",async()=>{
  for(const status of [408,429,502,503,504]) {
    resetRelayReadStateForTests();
    const urls:string[]=[];
    const rpc=new NetworkClient(base,(async(url:unknown,init:RequestInit)=>{
      urls.push(String(url));const body=JSON.parse(String(init.body));
      return String(url)==="/api/rpc"?unavailable(body,status):response(body,"0x2105");
    }) as typeof fetch,{relay:"/api/rpc"});
    assert.equal(await rpc.read("eth_chainId"),"0x2105",`HTTP ${status}`);
    assert.deepEqual(urls,["/api/rpc",READ_PROVIDERS[8453][0]]);
  }
});
test("auth failures, invalid evidence, RPC rejection and ambiguous fetch errors cannot trigger relay bypass",async()=>{
  const failures:[string,(body:any)=>Response][]=[
    ...[400,401,403,404,415,500].map(status=>[`HTTP ${status}`,(body:any)=>unavailable(body,status)] as [string,(body:any)=>Response]),
    ["malformed JSON",()=>new Response("{")],
    ["duplicate result",body=>new Response(`{"jsonrpc":"2.0","id":${body.id},"result":"0x2105","result":"0x1"}`)],
    ["wrong response ID",body=>response({...body,id:body.id+1},"0x2105")],
    ["wrong chain",body=>response(body,"0x1")],
    ["RPC rejection",body=>response(body,{code:-32005,message:"Rejected"},true)],
    ["ambiguous fetch failure",()=>{throw new TypeError("A network failure or an authentication redirect");}],
  ];
  for(const [name,fail] of failures) {
    resetRelayReadStateForTests();
    const urls:string[]=[];
    const rpc=new NetworkClient(base,(async(url:unknown,init:RequestInit)=>{
      urls.push(String(url));return fail(JSON.parse(String(init.body)));
    }) as typeof fetch,{relay:"/api/rpc"});
    await assert.rejects(rpc.read("eth_chainId"),undefined,name);
    assert.deepEqual(urls,["/api/rpc"],name);
  }
});
test("caller cancellation overrides a simultaneous relay 503 without starting direct reads",async()=>{
  const abort=new AbortController();const urls:string[]=[];
  const rpc=new NetworkClient(base,(async(url:unknown,init:RequestInit)=>{
    urls.push(String(url));abort.abort();return unavailable(JSON.parse(String(init.body)));
  }) as typeof fetch,{relay:"/api/rpc",signal:abort.signal});
  await assert.rejects(rpc.read("eth_chainId"),error=>error instanceof DOMException&&error.name==="AbortError");
  assert.deepEqual(urls,["/api/rpc"]);
});
test("relay cooldown coalesces concurrent clients, expires once, and stays isolated by chain",async context=>{
  let monotonic=1_000;context.mock.method(performance,"now",()=>monotonic);
  let baseProbes=0,ethereumProbes=0;
  const waiting:{body:any;resolve:(response:Response)=>void}[]=[];
  const fetcher=(async(url:unknown,init:RequestInit)=>{
    const body=JSON.parse(String(init.body));
    if(String(url)==="/api/rpc") {
      if(body.chainId===1) {ethereumProbes++;return response(body,"0x1");}
      baseProbes++;return new Promise<Response>(resolve=>waiting.push({body,resolve}));
    }
    assert.equal(String(url),READ_PROVIDERS[8453][0]);return response(body,"0x2105");
  }) as typeof fetch;
  const batch=()=>Array.from({length:6},()=>new NetworkClient(base,fetcher,{relay:"/api/rpc",signal:new AbortController().signal}).read("eth_chainId"));
  const first=batch();
  assert.equal(baseProbes,1);
  const initial=waiting.shift()!;initial.resolve(unavailable(initial.body));
  assert.deepEqual(await Promise.all(first),Array(6).fill("0x2105"));
  monotonic+=29_999;
  assert.deepEqual(await Promise.all(batch()),Array(6).fill("0x2105"));
  assert.equal(baseProbes,1);
  assert.equal(await new NetworkClient(ethereum,fetcher,{relay:"/api/rpc"}).read("eth_chainId"),"0x1");
  assert.equal(ethereumProbes,1);
  monotonic+=2;
  const retry=batch();
  assert.equal(baseProbes,2);
  const next=waiting.shift()!;next.resolve(unavailable(next.body));
  assert.deepEqual(await Promise.all(retry),Array(6).fill("0x2105"));
  assert.equal(baseProbes,2);
});
test("disqualifying relay evidence invalidates cooldown and defeats stale concurrent availability or success",async()=>{
  for(const failure of [401,403,"malformed"] as const) for(const availabilityFirst of [true,false]) {
    resetRelayReadStateForTests();
    let primed=false,drain=false;
    const calls:string[]=[];
    const waiting:{body:any;resolve:(response:Response)=>void}[]=[];
    const fetcher=(async(url:unknown,init:RequestInit)=>{
      const body=JSON.parse(String(init.body));calls.push(String(url));
      if(String(url)!=="/api/rpc") return response(body,"0x2105");
      if(!primed || drain) {primed=true;return response(body,"0x2105");}
      return new Promise<Response>(resolve=>waiting.push({body,resolve}));
    }) as typeof fetch;
    const client=()=>new NetworkClient(base,fetcher,{relay:"/api/rpc"});
    const rpc=client();await rpc.read("eth_chainId");
    // All four requests start in the same healthy generation. Settle every
    // request before assertions so a regression cannot leave test timers live.
    const reads=Array.from({length:4},()=>rpc.read("eth_chainId").then(()=>true,()=>false));
    const rejectEvidence=()=>waiting[1].resolve(failure==="malformed"?new Response("{"):unavailable(waiting[1].body,failure));
    if(availabilityFirst) {
      waiting[0].resolve(unavailable(waiting[0].body));await reads[0];
      rejectEvidence();await reads[1];
    } else {
      rejectEvidence();await reads[1];
      waiting[0].resolve(unavailable(waiting[0].body));await reads[0];
    }
    waiting[2].resolve(response(waiting[2].body,"0x2105"));await reads[2];
    waiting[3].resolve(unavailable(waiting[3].body));await reads[3];
    const results=await Promise.all(reads);
    const before=calls.length;drain=true;
    const fresh=Array.from({length:3},()=>client().read("eth_chainId"));
    const initialFreshCalls=calls.slice(before);
    await Promise.all(fresh);
    const scenario=`${failure}; availability first=${availabilityFirst}`;
    assert.deepEqual(results,[availabilityFirst,false,true,false],scenario);
    assert.deepEqual(initialFreshCalls,["/api/rpc"],`${scenario}: fresh readers must share a new relay probe`);
    assert.ok(calls.slice(before).every(url=>url==="/api/rpc"),`${scenario}: stale evidence must not revive direct fallback`);
  }
});
test("a cancelled concurrent relay request does not invalidate an independently observed outage",async()=>{
  let primed=false;const abort=new AbortController();const calls:string[]=[];
  const waiting:{body:any;resolve:(response:Response)=>void}[]=[];
  const fetcher=(async(url:unknown,init:RequestInit)=>{
    const body=JSON.parse(String(init.body));calls.push(String(url));
    if(String(url)!=="/api/rpc" || !primed) {primed=true;return response(body,"0x2105");}
    return new Promise<Response>(resolve=>waiting.push({body,resolve}));
  }) as typeof fetch;
  const rpc=new NetworkClient(base,fetcher,{relay:"/api/rpc"});
  await rpc.read("eth_chainId");
  const cancelled=new NetworkClient(base,fetcher,{relay:"/api/rpc",signal:abort.signal});
  const cancelledResult=assert.rejects(cancelled.read("eth_chainId"),error=>error instanceof DOMException&&error.name==="AbortError");
  const outage=rpc.read("eth_chainId");
  waiting[1].resolve(unavailable(waiting[1].body));await outage;
  abort.abort();waiting[0].resolve(unavailable(waiting[0].body));await cancelledResult;
  assert.equal(await rpc.read("eth_chainId"),"0x2105");
  assert.equal(calls.at(-1),READ_PROVIDERS[8453][0]);
});
test("cancelling the shared probe owner lets another reader retry the relay without direct fallback",async()=>{
  const abort=new AbortController();const urls:string[]=[];
  const fetcher=(async(url:unknown,init:RequestInit)=>{
    urls.push(String(url));const body=JSON.parse(String(init.body));
    if(urls.length===1) return new Promise<Response>((_resolve,reject)=>init.signal!.addEventListener("abort",()=>reject(new DOMException("Cancelled","AbortError")),{once:true}));
    return response(body,"0x2105");
  }) as typeof fetch;
  const owner=new NetworkClient(base,fetcher,{relay:"/api/rpc",signal:abort.signal});
  const peer=new NetworkClient(base,fetcher,{relay:"/api/rpc"});
  const ownerResult=assert.rejects(owner.read("eth_chainId"),/cancelled/i);
  const peerResult=peer.read("eth_chainId");
  abort.abort();
  await ownerResult;assert.equal(await peerResult,"0x2105");
  assert.deepEqual(urls,["/api/rpc","/api/rpc"]);
});
test("relay fallback never consumes a wrong-chain direct provider's balance",async()=>{
  const calls:{url:string;method:string}[]=[];
  const rpc=new NetworkClient(base,(async(url:unknown,init:RequestInit)=>{
    const body=JSON.parse(String(init.body));calls.push({url:String(url),method:body.method});
    if(String(url)==="/api/rpc") return unavailable(body);
    return response(body,body.method==="eth_chainId"?(String(url)===READ_PROVIDERS[8453][0]?"0x1":"0x2105"):"0x9");
  }) as typeof fetch,{relay:"/api/rpc"});
  assert.equal(await rpc.read("eth_getBalance",[FROM,"latest"]),"0x9");
  assert.deepEqual(calls.filter(call=>call.method==="eth_getBalance"),[{url:READ_PROVIDERS[8453][1],method:"eth_getBalance"}]);
});
test("successful direct read fallback never changes the single configured submission path",async()=>{
  const calls:{url:string;method:string}[]=[];
  const rpc=new NetworkClient(base,(async(url:unknown,init:RequestInit)=>{
    const body=JSON.parse(String(init.body));calls.push({url:String(url),method:body.method});
    return String(url)==="/api/rpc"?unavailable(body):response(body,"0x2105");
  }) as typeof fetch,{relay:"/api/rpc"});
  assert.equal(await rpc.read("eth_chainId"),"0x2105");
  await assert.rejects(rpc.broadcast(sign(prepare("outage-read-then-submit",8453))),error=>error instanceof RpcError&&error.submission&&/Submission is unconfirmed/.test(error.message));
  assert.deepEqual(calls.filter(call=>call.method==="eth_sendRawTransaction"),[{url:"/api/rpc",method:"eth_sendRawTransaction"}]);
});
test("direct fallback uses direct request deadlines while explicit timeout overrides remain honored",async context=>{
  const deadlines:number[]=[];const nativeSetTimeout=globalThis.setTimeout;
  context.mock.method(globalThis,"setTimeout",(callback:any,delay:number,...args:any[])=>{deadlines.push(delay);return nativeSetTimeout(callback,delay,...args);});
  const fetcher=(async(url:unknown,init:RequestInit)=>{
    const body=JSON.parse(String(init.body));return String(url)==="/api/rpc"?unavailable(body):response(body,"0x2105");
  }) as typeof fetch;
  assert.equal(await new NetworkClient(base,fetcher,{relay:"/api/rpc"}).read("eth_chainId"),"0x2105");
  assert.deepEqual(deadlines,[52_000,12_000]);
  resetRelayReadStateForTests();deadlines.length=0;
  assert.equal(await new NetworkClient(base,fetcher,{relay:"/api/rpc",timeoutMs:777}).read("eth_chainId"),"0x2105");
  assert.deepEqual(deadlines,[777,777]);
});

test("browser default fetch is invoked without a transport receiver for RPC reads",async context=>{
  const calls:{receiver:unknown;url:string}[]=[];
  context.mock.method(globalThis,"fetch",function(this:unknown,url:RequestInfo|URL,init?:RequestInit){
    calls.push({receiver:this,url:String(url)});
    // Native browser fetch rejects an unrelated class instance as its receiver.
    // Node fetch and the arrow-function test doubles do not enforce that check.
    if(this!==undefined && this!==globalThis) throw new TypeError("Illegal invocation");
    return Promise.resolve(response(JSON.parse(String(init?.body)),"0x2105"));
  });
  const rpc=new NetworkClient(base,undefined,{relay:"/api/rpc"});
  assert.equal(await rpc.read("eth_chainId"),"0x2105");
  assert.deepEqual(calls,[{receiver:undefined,url:"/api/rpc"}]);
});
test("browser activity discovery is invoked without a transport receiver and preserves request policy",async()=>{
  const calls:{receiver:unknown;url:string;init:RequestInit|undefined}[]=[];
  const transactionHash=blockHash();
  const fetcher=function(this:unknown,url:RequestInfo|URL,init?:RequestInit){
    calls.push({receiver:this,url:String(url),init});
    if(this!==undefined && this!==globalThis) throw new TypeError("Illegal invocation");
    return Promise.resolve(new Response(JSON.stringify({chainId:8453,address:FROM,hashes:[transactionHash],partial:true})));
  } as typeof fetch;
  const rpc=new NetworkClient(base,fetcher,{relay:"/api/rpc"});
  assert.deepEqual(await rpc.activityCandidates(FROM),{hashes:[transactionHash],partial:true,notice:undefined});
  assert.equal(calls.length,1);
  assert.equal(calls[0].receiver,undefined);
  assert.equal(calls[0].url,`/api/activity?chainId=8453&address=${encodeURIComponent(FROM)}`);
  assert.equal(calls[0].init?.credentials,"same-origin");
  assert.equal(calls[0].init?.mode,"cors");
  assert.equal(calls[0].init?.redirect,"error");
  assert.equal(calls[0].init?.referrerPolicy,"no-referrer");
});
test("browser relay POST retains a matching Origin with no-referrer",async()=>{
  resetRelayStateForTests();
  const origin="https://wallet.example";
  const observed:{origin:string;init:RequestInit}[]=[];
  let upstreamCalls=0;
  const browserFetch=(async(url:RequestInfo|URL,init:RequestInit={})=>{
    assert.equal(String(url),"/api/rpc");
    const headers=new Headers(init.headers);
    // Fetch's append-Origin algorithm makes non-CORS POST + no-referrer
    // opaque. Model that browser behavior instead of injecting a matching
    // Origin unconditionally, as the in-process acceptance check does.
    const requestOrigin=init.mode!=="cors" && init.referrerPolicy==="no-referrer" && !["GET","HEAD"].includes(init.method??"GET")?"null":origin;
    headers.set("Origin",requestOrigin);
    headers.set("Sec-Fetch-Site","same-origin");
    observed.push({origin:requestOrigin,init});
    return relayRpc(new Request(new URL(String(url),origin),{...init,headers}),async(_input,options)=>{
      upstreamCalls++;
      return response(JSON.parse(String(options?.body)),"0x2105");
    });
  }) as typeof fetch;
  const rpc=new NetworkClient(base,browserFetch,{relay:"/api/rpc"});
  assert.equal(await rpc.read("eth_chainId"),"0x2105");
  assert.equal(upstreamCalls,1);
  assert.equal(observed.length,1);
  assert.equal(observed[0].origin,origin);
  assert.equal(observed[0].init.credentials,"same-origin");
  assert.equal(observed[0].init.redirect,"error");
  assert.equal(observed[0].init.referrerPolicy,"no-referrer");
});

test("read failover verifies each fixed endpoint's chain before using its values",async()=>{
  const calls:string[]=[];
  const fetcher=(async(url:unknown,init:RequestInit)=>{const body=JSON.parse(init.body as string);calls.push(`${url}:${body.method}`);
    if(String(url)===READ_PROVIDERS[1][0]) throw new Error("offline");
    return response(body,body.method==="eth_chainId"?"0x1":"0xa");
  }) as typeof fetch;
  const rpc=new NetworkClient(ethereum,fetcher);
  assert.equal(await rpc.read("eth_getBalance",[FROM,"latest"]),"0xa");
  assert.deepEqual(calls,[`${READ_PROVIDERS[1][0]}:eth_chainId`,`${READ_PROVIDERS[1][1]}:eth_chainId`,`${READ_PROVIDERS[1][1]}:eth_getBalance`]);
  assert.equal(rpc.connectionInfo().provider,"Alchemy public RPC");
});
test("a wrong-chain primary never supplies the balance and falls back to a verified endpoint",async()=>{
  const queried:string[]=[];
  const fetcher=(async(url:unknown,init:RequestInit)=>{const body=JSON.parse(init.body as string);if(body.method!=="eth_chainId") queried.push(String(url));return response(body,body.method==="eth_chainId"?(String(url)===READ_PROVIDERS[8453][0]?"0x1":"0x2105"):"0x5");}) as typeof fetch;
  assert.equal(await new NetworkClient(base,fetcher).read("eth_getBalance",[FROM,"latest"]),"0x5");assert.deepEqual(queried,[READ_PROVIDERS[8453][1]]);
});
test("read failures are availability errors, never unknown financial outcomes or zero balances",async()=>{
  const rpc=new NetworkClient(base,(async()=>{throw new TypeError("Network unavailable");}) as typeof fetch);
  const status=await rpc.balanceStatus(FROM);assert.equal(status.status,"unavailable");assert.equal(status.snapshot,null);assert.match(status.error!,/No transaction was submitted/);assert.doesNotMatch(status.error!,/outcome unknown|Submission is unconfirmed/);
});
test("relay mode retains same-origin gate credentials and never falls back direct after an uncertain send",async()=>{
  const calls:{url:string;init:RequestInit}[]=[];
  const fetcher=(async(url:unknown,init:RequestInit)=>{calls.push({url:String(url),init});throw new Error("lost response");}) as typeof fetch;
  const rpc=new NetworkClient(base,fetcher,{relay:"/api/rpc"});
  await assert.rejects(rpc.broadcast(sign(prepare("relay-single",8453))),error=>error instanceof RpcError&&error.submission&&/Submission is unconfirmed/.test(error.message));
  assert.deepEqual(calls.map(call=>call.url),["/api/rpc"]);
  // Assert outside the rejecting fetch: a swallowed assertion must not be
  // mistaken for the expected uncertain-submission failure.
  assert.equal(calls[0].init.credentials,"same-origin");
  assert.equal(calls[0].init.mode,"cors");
  assert.equal(calls[0].init.redirect,"error");
  assert.equal(calls[0].init.referrerPolicy,"no-referrer");
  const body=JSON.parse(String(calls[0].init.body));
  assert.equal(body.chainId,8453);
  assert.equal(body.method,"eth_sendRawTransaction");
});
test("direct broadcast uses a single sequencer attempt and does not query read fallbacks",async()=>{
  const urls:string[]=[];const rpc=new NetworkClient(base,(async(url:unknown,init:RequestInit)=>{urls.push(String(url));assert.equal(init.credentials,"omit");throw new Error("offline");}) as typeof fetch);
  await assert.rejects(rpc.broadcast(sign(prepare("direct-single",8453))));assert.deepEqual(urls,[base.broadcast_url]);
});
test("read-only preview remains available without ETH but prepare still rejects the unsimulated transfer",async()=>{
  const calls:string[]=[];const block={hash:blockHash(),number:"0x64",timestamp:hexQuantity(Math.floor(Date.now()/1000)),baseFeePerGas:"0x3b9aca00"};
  const fetcher=(async(_url:unknown,init:RequestInit)=>{const body=JSON.parse(init.body as string);calls.push(body.method);
    switch(body.method) {
      case "eth_chainId":return response(body,"0x2105");
      case "eth_getBlockByNumber":return response(body,block);
      case "eth_maxPriorityFeePerGas":return response(body,"0x5f5e100");
      case "eth_getBalance":return response(body,"0x0");
      case "eth_getTransactionCount":return response(body,"0x0");
      case "eth_getCode":return response(body,"0x");
      case "eth_estimateGas":return response(body,{code:-32000,message:"insufficient funds"},true);
      case "eth_call": {
        const selector=body.params[0].data.slice(0,10);
        if(selector==="0x70a08231")return response(body,word(0));
        if(selector==="0xf1c7a58b")return response(body,word(1000));
        if(selector==="0x275aedd2")return response(body,word(100));
        if(selector==="0x313ce567")return response(body,word(6));
        return response(body,{code:-32000,message:"execution reverted"},true);
      }
      default:throw new Error(`Unexpected ${body.method}`);
    }
  }) as typeof fetch;
  const rpc=new NetworkClient(base,fetcher);
  const preview=await rpc.preview({from:FROM,to:TO,asset:"USDC",amount:"1"});
  assert.equal(preview.estimate_source,"conservative");assert.equal(preview.gas_limit,150_000);assert.equal(preview.l1_fee_estimate_wei,"1000");assert.equal(preview.operator_fee_estimate_wei,"100");
  assert.equal(BigInt(preview.estimated_total_fee_wei),BigInt(preview.estimated_execution_fee_wei)+1100n);
  assert.equal(BigInt(preview.fee_reserve_wei),BigInt(preview.max_execution_fee_wei)+2200n);
  assert.equal(preview.shortages.length,2);assert.ok(preview.warnings.some(w=>w.includes("conservative")));assert.ok(!calls.includes("eth_sendRawTransaction"));assert.ok(!calls.includes("eth_getTransactionCount"));
  await assert.rejects(rpc.quote("unfunded",FROM,TO,"USDC","1"));
});
test("a cancelled draft preview performs no reads and never manufactures a fee on unavailable networks",async()=>{
  let calls=0;const rpc=new NetworkClient(base,(async()=>{calls++;throw new Error("offline");}) as typeof fetch);
  const aborted=new AbortController();aborted.abort();await assert.rejects(rpc.preview({from:FROM,to:TO,asset:"ETH",amount:"0.01"},aborted.signal),/cancelled/);assert.equal(calls,0);
  await assert.rejects(rpc.preview({from:FROM,to:TO,asset:"ETH",amount:"0.01"}));assert.equal(calls,2);
});
