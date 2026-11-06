import type { ChainId, ConnectionInfo, Network } from "./types";
import { quantity } from "./units";
import { parseStrictJson } from "./strict-json";

export const READ_PROVIDERS = {
  1: ["https://ethereum-rpc.publicnode.com", "https://eth-mainnet.g.alchemy.com/public"],
  8453: ["https://base-rpc.publicnode.com", "https://mainnet.base.org"],
} as const;
export interface TransportOptions { relay?: "/api/rpc" | null; signal?: AbortSignal; timeoutMs?: number }
export class RpcError extends Error {
  constructor(message: string, public code: number | null = null, public retryable = false, public submission = false, public httpStatus: number | null = null) { super(message); this.name = "RpcError"; }
}
const LIMIT = 1_048_576;
const RELAY_COOLDOWN_MS = 30_000;
const RELAY_AVAILABILITY_STATUSES = new Set([408, 429, 502, 503, 504]);
const RELAY_UNAVAILABLE = Symbol("relay unavailable for reads");
type RelayProbeResult = { retry: true } | { error: unknown };
interface RelayReadState { healthy: boolean; until: number; revision: number; probe: Promise<RelayProbeResult> | null }
const newRelayState = (): RelayReadState => ({ healthy: false, until: 0, revision: 0, probe: null });
// Availability state is bounded to the two supported chains and shared by
// preview clones. No addresses, responses, credentials or signing state live here.
const relayReads: Record<ChainId, RelayReadState> = { 1: newRelayState(), 8453: newRelayState() };
function waitForRelayProbe(probe: Promise<RelayProbeResult>, signal?: AbortSignal): Promise<RelayProbeResult> {
  if (!signal) return probe;
  return new Promise((resolve, reject) => {
    const cancelled = () => { signal.removeEventListener("abort", cancelled); reject(new DOMException("Request cancelled", "AbortError")); };
    if (signal.aborted) { cancelled(); return; }
    signal.addEventListener("abort", cancelled, { once: true });
    void probe.then(result => { signal.removeEventListener("abort", cancelled); resolve(result); });
  });
}
/** Deterministic isolation between transport tests; no persisted health state. */
export function resetRelayReadStateForTests(): void { relayReads[1] = newRelayState(); relayReads[8453] = newRelayState(); }
export function relayConfiguration(): "/api/rpc" | null {
  if (typeof document === "undefined") return null;
  const configured = document.querySelector<HTMLMetaElement>('meta[name="wallet-rpc-relay"]')?.content;
  if (configured !== undefined && configured !== "/api/rpc") throw new Error("Invalid wallet relay configuration.");
  return configured ?? null;
}
export async function boundedJson(response: Response): Promise<unknown> {
  const size = response.headers.get("content-length");
  if (size !== null && (!/^\d+$/.test(size) || Number(size) > LIMIT)) throw new RpcError("The network response exceeded its size limit.", null, true);
  if (!response.body) throw new RpcError("The network returned an empty response.", null, true);
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = []; let total = 0;
  for (;;) {
    const part = await reader.read(); if (part.done) break;
    total += part.value.byteLength;
    if (total > LIMIT) { await reader.cancel(); throw new RpcError("The network response exceeded its size limit.", null, true); }
    chunks.push(part.value);
  }
  const bytes = new Uint8Array(total); let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk,offset); offset += chunk.length; }
  try { return parseStrictJson(new TextDecoder("utf-8",{fatal:true}).decode(bytes)); }
  catch { throw new RpcError("The network returned malformed or ambiguous JSON.",null,true); }
}
function label(endpoint: string): string {
  if (endpoint.includes("publicnode")) return "PublicNode";
  if (endpoint.includes("alchemy")) return "Alchemy public RPC";
  if (endpoint.includes("base.org")) return "Base public RPC";
  return "Wallet mainnet relay";
}
export class RpcTransport {
  private sequence = 0;
  private preferred = 0;
  private verified = new Map<string,Promise<void>>();
  readonly relay: "/api/rpc" | null;
  readonly fetcher: typeof fetch;
  private info: ConnectionInfo;
  constructor(readonly network: Network, fetcher: typeof fetch, readonly options: TransportOptions = {}) {
    // Native browser fetch rejects a transport instance as its receiver. Keep
    // the injected function in a closure and invoke it without a receiver.
    this.fetcher = (input, init) => fetcher(input, init);
    this.relay = options.relay === undefined ? relayConfiguration() : options.relay;
    this.info = {chain_id:network.chain_id,provider:null,endpoint:null,transport:null,checked_at:null,failed_attempts:0};
  }
  connectionInfo(): ConnectionInfo { return {...this.info}; }
  private async raw(endpoint: string, method: string, params: unknown[], submission = false): Promise<unknown> {
    if (this.options.signal?.aborted) throw new DOMException("Request cancelled","AbortError");
    const relay = endpoint === "/api/rpc";
    const abort = new AbortController();
    const cancel = () => abort.abort(); this.options.signal?.addEventListener("abort",cancel,{once:true});
    const timer = setTimeout(cancel,this.options.timeoutMs ?? (relay ? 52_000 : 12_000));
    const id = ++this.sequence;
    try {
      // The relay URL is fixed and same-origin. CORS mode preserves its Origin
      // header under no-referrer; non-CORS POST can instead send Origin:null.
      const response = await this.fetcher(endpoint,{
        method:"POST",headers:{"Content-Type":"application/json","Accept":"application/json"},
        body:JSON.stringify({...(relay?{chainId:this.network.chain_id}:{}),jsonrpc:"2.0",id,method,params}),
        signal:abort.signal,credentials:relay?"same-origin":"omit",redirect:"error",cache:"no-store",referrerPolicy:"no-referrer",mode:"cors",
      });
      if (!response.ok) throw new RpcError(response.status === 429 ? "The RPC provider is rate limiting reads." : `The RPC connection returned HTTP ${response.status}.`,null,true,submission,response.status);
      const message = await boundedJson(response) as {jsonrpc?:unknown;id?:unknown;error?:{code?:unknown};result?:unknown}|null;
      if (!message || Array.isArray(message) || message.jsonrpc!=="2.0" || message.id!==id || Object.hasOwn(message,"error")===Object.hasOwn(message,"result")) throw new RpcError("The provider returned a mismatched JSON-RPC response.",null,true,submission);
      if (Object.hasOwn(message,"error")) {
        const code = Number.isSafeInteger(message.error?.code) ? message.error!.code as number : null;
        // No provider-supplied prose is treated as an instruction or shown in UI.
        throw new RpcError(`The RPC provider rejected this read${code===null?"":` (code ${code})`}.`,code,[-32601,-32602,-32005,-32016].includes(code ?? 0),submission);
      }
      const declared = response.headers.get("X-Wallet-Rpc-Provider");
      const known = Object.values(READ_PROVIDERS).flat().find(value => value === declared || new URL(value).hostname === declared);
      this.info = {...this.info,provider:label(known ?? endpoint),endpoint:known ?? endpoint,transport:relay?"relay":"direct",checked_at:Math.floor(Date.now()/1000)};
      return message.result;
    } catch (error) {
      if (this.options.signal?.aborted) throw new DOMException("Request cancelled","AbortError");
      if (error instanceof RpcError) {
        if (submission) throw new RpcError("Submission is unconfirmed. The saved transaction may have reached the network; reconcile before retrying.",error.code,false,true,error.httpStatus);
        throw error;
      }
      throw new RpcError(submission ? "Submission is unconfirmed. The saved transaction may have reached the network; reconcile before retrying." : "The RPC connection is unavailable or blocked. No transaction was submitted by this read.",null,true,submission);
    } finally { clearTimeout(timer); this.options.signal?.removeEventListener("abort",cancel); }
  }
  private async assertEndpoint(endpoint: string): Promise<void> {
    let promise = this.verified.get(endpoint);
    if (!promise) {
      promise = this.raw(endpoint,"eth_chainId",[]).then(result => {
        if (quantity(result)!==BigInt(this.network.chain_id)) throw new RpcError("The provider returned the wrong network. Reads from this endpoint are blocked.",null,true);
      });
      this.verified.set(endpoint,promise);
      promise.catch(()=>this.verified.delete(endpoint));
    }
    await promise;
  }
  private async readEndpoint(endpoint: string, method: string, params: unknown[]): Promise<unknown> {
    if (method !== "eth_chainId") await this.assertEndpoint(endpoint);
    const result = await this.raw(endpoint, method, params);
    if (method === "eth_chainId" && quantity(result) !== BigInt(this.network.chain_id)) throw new RpcError("The provider returned the wrong network. Reads from this endpoint are blocked.", null, true);
    return result;
  }
  private async readRelay(method: string, params: unknown[]): Promise<unknown> {
    const state = relayReads[this.network.chain_id];
    for (;;) {
      if (this.options.signal?.aborted) throw new DOMException("Request cancelled", "AbortError");
      if (state.until > performance.now()) return RELAY_UNAVAILABLE;
      if (state.probe) {
        const outcome = await waitForRelayProbe(state.probe, this.options.signal);
        if ("error" in outcome) throw outcome.error;
        continue;
      }
      // Only one cold/recovery request probes the relay. Other readers wait
      // for its outcome; they cannot assume an outage or bypass an auth error.
      let finishProbe: ((result: RelayProbeResult) => void) | undefined;
      if (!state.healthy) state.probe = new Promise(resolve => { finishProbe = resolve; });
      const revision = state.revision;
      let outcome: RelayProbeResult = { retry: true };
      try {
        const result = await this.readEndpoint(this.relay!, method, params);
        if (state.revision === revision) { state.healthy = true; state.until = 0; }
        return result;
      } catch (error) {
        if (error instanceof RpcError && error.retryable) this.info.failed_attempts++;
        // Never infer availability from malformed evidence, a JSON-RPC
        // rejection, an auth failure, cancellation, or an ambiguous TypeError.
        if (error instanceof RpcError && !error.submission && error.httpStatus !== null && RELAY_AVAILABILITY_STATUSES.has(error.httpStatus)) {
          if (state.revision === revision) {
            state.revision++; state.healthy = false; state.until = performance.now() + RELAY_COOLDOWN_MS;
          }
          // A concurrent availability result may use an existing cooldown,
          // but cannot revive or extend it after newer disqualifying evidence.
          if (state.until > performance.now()) return RELAY_UNAVAILABLE;
          outcome = { error };
          throw error;
        }
        if (!this.options.signal?.aborted) {
          state.revision++; state.healthy = false; state.until = 0;
          outcome = { error };
        }
        throw error;
      } finally {
        if (finishProbe) { state.probe = null; finishProbe(outcome); }
      }
    }
  }
  async read(method: string,params:unknown[]):Promise<unknown> {
    if (this.relay) {
      const result = await this.readRelay(method, params);
      if (result !== RELAY_UNAVAILABLE) return result;
    }
    const providers = [...READ_PROVIDERS[this.network.chain_id]];
    let last: unknown;
    for (let offset=0;offset<providers.length;offset++) {
      const index=(this.preferred+offset)%providers.length; const endpoint=providers[index];
      try {
        const result=await this.readEndpoint(endpoint,method,params);
        this.preferred=index;return result;
      } catch(error) {
        if (!(error instanceof RpcError) || !error.retryable) throw error;
        this.info.failed_attempts++;last=error;
      }
    }
    throw last ?? new RpcError("No mainnet read provider is available. Your saved wallet and balances are unchanged.");
  }
  // Deliberately a SINGLE path/attempt. A failed relay submission never triggers
  // automatic direct fallback, even if the read providers are unavailable.
  broadcast(raw:string):Promise<unknown> { return this.raw(this.relay ?? this.network.broadcast_url,"eth_sendRawTransaction",[raw],true); }
  async activityCandidates(account:string):Promise<{hashes:string[];partial:boolean;notice?:string}> {
    if (!this.relay) throw new RpcError("A history index is unavailable in direct mode. Recent token scans and transaction-hash lookup remain available.");
    const abort = new AbortController();const timer=setTimeout(()=>abort.abort(),10_000);
    try {
      const response=await this.fetcher(`/api/activity?chainId=${this.network.chain_id}&address=${encodeURIComponent(account)}`,{credentials:"same-origin",redirect:"error",cache:"no-store",referrerPolicy:"no-referrer",mode:"cors",signal:abort.signal});
      if(!response.ok) throw new RpcError("The history index is unavailable. Recent scans and transaction-hash lookup remain available.");
      const result=await boundedJson(response) as {chainId?:unknown;address?:unknown;hashes?:unknown;partial?:unknown;notice?:unknown};
      if(result.chainId!==this.network.chain_id || typeof result.address!=="string" || result.address.toLowerCase()!==account.toLowerCase() || !Array.isArray(result.hashes) || result.hashes.length>100 || result.hashes.some(h=>typeof h!=="string"||!/^0x[0-9a-fA-F]{64}$/.test(h))) throw new RpcError("The history index returned invalid candidate data.");
      return {hashes:result.hashes as string[],partial:result.partial!==false,notice:result.notice?"History index coverage is limited; confirmed balances are independent of this list.":undefined};
    } finally {clearTimeout(timer);}
  }
}
