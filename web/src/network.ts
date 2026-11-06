import type { Asset, BalanceSnapshot, BalanceStatus, Block, ConnectionInfo, FeeDraft, FeePreview, Network, PaymentRequest, PreparedPayment, SignedPayment } from "./types";
import { abiUint, address, decimal, formatUnits, hash, hexQuantity, networkAsset, quantity, safeNumber, sameAddress, units } from "./units";
import { RpcError, RpcTransport, type TransportOptions } from "./transport";
export { RpcError } from "./transport";

const ORACLE = "0x420000000000000000000000000000000000000F";
const L1_UPPER = "0xf1c7a58b";
const OPERATOR = "0x275aedd2";
const READ_METHODS = new Set(["eth_chainId", "eth_getBlockByNumber", "eth_getBalance", "eth_getTransactionCount", "eth_getCode", "eth_call", "eth_estimateGas", "eth_maxPriorityFeePerGas", "eth_getTransactionReceipt", "eth_getTransactionByHash", "eth_getLogs"]);
type Fetcher = typeof fetch;
type RpcTransaction = { from: string; to: string; value: string; data: string; maxFeePerGas?: string; maxPriorityFeePerGas?: string; gas?: string };
export interface Quote { request: PaymentRequest; snapshot: BalanceSnapshot }
export function rpcTransaction(network: Network, from: string, to: string, asset: Asset, amountUnits: bigint): RpcTransaction {
  address(from); address(to);
  const spec = networkAsset(network, asset);
  return spec.contract
    ? { from, to: spec.contract, value: "0x0", data: `0xa9059cbb${to.slice(2).toLowerCase().padStart(64, "0")}${amountUnits.toString(16).padStart(64, "0")}` }
    : { from, to, value: hexQuantity(amountUnits), data: "0x" };
}
export function minimumGas(data: string): bigint {
  // Reserve the stricter EIP-7623 calldata floor for both networks.
  let gas = 21_000n;
  for (let i = 2; i < data.length; i += 2) gas += data.slice(i, i + 2) === "00" ? 10n : 40n;
  return gas;
}
export class NetworkClient {
  private transport: RpcTransport;
  constructor(public readonly network: Network, private readonly fetcher: Fetcher = fetch, private readonly transportOptions: TransportOptions = {}) {
    const allowed = network.chain_id === 1
      ? { rpc: "https://ethereum-rpc.publicnode.com", broadcast: "https://ethereum-rpc.publicnode.com" }
      : network.chain_id === 8453
        ? { rpc: "https://mainnet.base.org", broadcast: "https://mainnet-sequencer.base.org" }
        : null;
    if (!allowed || network.rpc_url !== allowed.rpc || network.broadcast_url !== allowed.broadcast) throw new Error("The network registry is not an approved mainnet configuration.");
    this.transport = new RpcTransport(network, fetcher, transportOptions);
  }
  connectionInfo(): ConnectionInfo { return this.transport.connectionInfo(); }
  activityCandidates(account: string) { address(account); return this.transport.activityCandidates(account); }
  async read(method: string, params: unknown[] = []): Promise<unknown> {
    if (!READ_METHODS.has(method)) throw new Error("Unsupported RPC operation.");
    return this.transport.read(method, params);
  }
  async verifyChain(): Promise<void> {
    if (quantity(await this.read("eth_chainId")) !== BigInt(this.network.chain_id)) throw new Error("The provider returned the wrong network. Sending is blocked.");
  }
  async block(tag: string, requireFresh = false, fullTransactions = false): Promise<Block> {
    const value = await this.read("eth_getBlockByNumber", [tag, fullTransactions]) as Block | null;
    if (!value || typeof value !== "object") throw new Error("The requested canonical block is unavailable.");
    hash(value.hash); safeNumber(quantity(value.number)); if(value.baseFeePerGas!==undefined) quantity(value.baseFeePerGas);
    const time = safeNumber(quantity(value.timestamp));
    if (requireFresh && (Math.abs(Math.floor(Date.now() / 1000) - time) > 300)) throw new Error("The provider’s latest block is stale, or this device’s clock is incorrect.");
    if (tag.startsWith("0x") && quantity(tag) !== quantity(value.number)) throw new Error("The provider returned the wrong block height.");
    return value;
  }
  private tag(block: Block) { return { blockHash: hash(block.hash), requireCanonical: true }; }
  async assertCanonical(block: Block): Promise<void> {
    if (hash((await this.block(block.number)).hash) !== hash(block.hash)) throw new Error("The chain changed while reading. Refresh before continuing.");
  }
  async balanceAt(from: string, block: Block): Promise<BalanceSnapshot> {
    address(from);
    const tag = this.tag(block);
    const values = await Promise.all(this.network.assets.map(async spec => {
      const value = spec.contract
        ? abiUint(await this.read("eth_call", [{ to: spec.contract, data: `0x70a08231${from.slice(2).padStart(64, "0")}` }, tag]))
        : quantity(await this.read("eth_getBalance", [from, tag]));
      return [spec.symbol, value.toString()] as const;
    }));
    const balances = Object.fromEntries(values) as BalanceSnapshot["balances"];
    return { chain_id: this.network.chain_id, address: from, block_number: safeNumber(quantity(block.number)), block_hash: hash(block.hash), balances, observed_at: Math.floor(Date.now() / 1000) };
  }
  async balances(from: string): Promise<BalanceSnapshot> {
    await this.verifyChain();
    const block = await this.block("latest", true);
    const snapshot = await this.balanceAt(from, block);
    await this.assertCanonical(block);
    return snapshot;
  }
  async balanceStatus(from: string): Promise<BalanceStatus> {
    try {
      const snapshot = await this.balances(from);
      return {status:"fresh",snapshot,provider:this.connectionInfo().provider,observed_at:Math.floor(Date.now()/1000),error:null};
    } catch(error) {
      return {status:"unavailable",snapshot:null,provider:this.connectionInfo().provider,observed_at:Math.floor(Date.now()/1000),error:error instanceof Error?error.message:"Balances could not be read. The saved observation is unchanged."};
    }
  }
  /** Read-only draft estimate. No nonce reservation, journal write or signing.
   * Lack of funds affects the explanation, not availability of a cost preview.
   * prepare/quote still require successful real transfer simulation and funds. */
  async preview(draft: FeeDraft, signal?: AbortSignal): Promise<FeePreview> {
    if (signal) return new NetworkClient(this.network,this.fetcher,{...this.transportOptions,signal}).preview(draft);
    address(draft.from);address(draft.to);
    const spec=networkAsset(this.network,draft.asset);
    const amount=units(draft.amount,spec.decimals);
    if(amount===0n) throw new Error("Enter an amount greater than zero to estimate the transfer fee.");
    await this.verifyChain();
    const block=await this.block("latest",true);
    const tx=rpcTransaction(this.network,draft.from,draft.to,draft.asset,amount);
    const [priorityResult,simulation,balanceResult]=await Promise.allSettled([
      this.read("eth_maxPriorityFeePerGas"),this.simulate(tx,draft.asset),this.balanceAt(draft.from,block),
    ]);
    if(this.transportOptions.signal?.aborted) throw new DOMException("Request cancelled","AbortError");
    if(priorityResult.status!=="fulfilled") throw priorityResult.reason;
    const priority=quantity(priorityResult.value),base=quantity(block.baseFeePerGas),maxFee=base*2n+priority;
    const warnings:string[]=[];const shortages:string[]=[];
    let gas:bigint;let source:FeePreview["estimate_source"]="simulation";
    if(simulation.status==="fulfilled") gas=simulation.value;
    else {
      source="conservative";
      if(draft.asset==="ETH") {
        let code:unknown=null;
        try {code=await this.read("eth_getCode",[draft.to,this.tag(block)]);} catch { /* conservative cap below */ }
        gas=code==="0x"?21_000n:240_000n;
      } else gas=120_000n;
      warnings.push("This is a conservative cost estimate because the exact transfer could not be simulated. Review still requires a successful simulation and sufficient funds.");
    }
    const floor=minimumGas(tx.data);
    if(gas<floor) gas=floor;
    const padded=(gas*125n+99n)/100n;
    const gasLimit=padded>floor?padded:floor;
    if(gasLimit>300_000n || maxFee===0n || maxFee>500_000_000_000n) throw new Error("Current transfer costs exceed this wallet’s supported limits.");
    const extra=await this.additionalFees(Number(gasLimit),block);
    const execution=gas*(base+priority),maximum=gasLimit*maxFee,reserve=maximum+2n*(extra.l1+extra.operator);
    if(balanceResult.status==="fulfilled") {
      const balances=balanceResult.value.balances;
      if(decimal(balances[draft.asset])<amount) shortages.push(`Insufficient ${draft.asset}: available ${formatUnits(balances[draft.asset],spec.decimals)} on ${this.network.network}.`);
      const nativeNeeded=reserve+(draft.asset==="ETH"?amount:0n);
      if(decimal(balances.ETH)<nativeNeeded) shortages.push(`ETH needed for amount and fee reserve: ${formatUnits(nativeNeeded,18)}; available ${formatUnits(balances.ETH,18)} on ${this.network.network}.`);
    } else warnings.push("The balance check is unavailable. This estimate does not confirm that the wallet can fund the transfer.");
    if(reserve>50_000_000_000_000_000n) warnings.push("The reviewed fee reserve would exceed this wallet’s 0.05 ETH policy limit. This transfer cannot be prepared at current costs.");
    await this.assertCanonical(block);
    return {chain_id:this.network.chain_id,asset:draft.asset,estimated_execution_fee_wei:execution.toString(),
      l1_fee_estimate_wei:extra.l1.toString(),operator_fee_estimate_wei:extra.operator.toString(),
      estimated_total_fee_wei:(execution+extra.l1+extra.operator).toString(),max_execution_fee_wei:maximum.toString(),fee_reserve_wei:reserve.toString(),
      gas_limit:Number(gasLimit),max_fee_per_gas:maxFee.toString(),max_priority_fee_per_gas:priority.toString(),provider:this.connectionInfo().provider,
      estimated_at:Math.floor(Date.now()/1000),estimate_source:source,warnings,shortages};
  }
  private async noDelegation(from: string, block: Block): Promise<void> {
    const [canonical, pending] = await Promise.all([
      this.read("eth_getCode", [from, this.tag(block)]),
      this.read("eth_getCode", [from, "pending"]),
    ]);
    if (canonical !== "0x" || pending !== "0x") throw new Error("This account has contract code or delegation. This wallet supports ordinary accounts only.");
  }
  async nonce(from: string): Promise<number> {
    const [latest, pending] = await Promise.all([
      this.read("eth_getTransactionCount", [from, "latest"]),
      this.read("eth_getTransactionCount", [from, "pending"]),
    ]);
    const latestCount = quantity(latest), pendingCount = quantity(pending);
    if (pendingCount !== latestCount) throw new Error("This account has an external pending transaction, or the provider is inconsistent. Reconcile it before sending.");
    return safeNumber(latestCount);
  }
  private async tokenChecks(asset: Asset, block: Block): Promise<void> {
    const spec = networkAsset(this.network, asset);
    if (!spec.contract) return;
    const [code, decimals] = await Promise.all([
      this.read("eth_getCode", [spec.contract, this.tag(block)]),
      this.read("eth_call", [{ to: spec.contract, data: "0x313ce567" }, this.tag(block)]),
    ]);
    if (typeof code !== "string" || !/^0x(?:[0-9a-fA-F]{2})+$/.test(code) || abiUint(decimals) !== BigInt(spec.decimals)) throw new Error("The supported token contract could not be verified on this network.");
  }
  private async simulate(tx: RpcTransaction, asset: Asset): Promise<bigint> {
    const [callResult, gas] = await Promise.all([
      this.read("eth_call", [tx, "pending"]),
      this.read("eth_estimateGas", [tx, "pending"]),
    ]);
    if (asset !== "ETH" && abiUint(callResult) !== 1n) throw new Error("The token transfer simulation did not return success.");
    if (asset === "ETH" && (typeof callResult !== "string" || !/^0x(?:[0-9a-fA-F]{2})*$/.test(callResult))) throw new Error("Invalid transfer simulation response.");
    const estimate = quantity(gas);
    if (estimate < 21_000n || estimate > 300_000n) throw new Error("This transfer requires gas outside the wallet’s supported range.");
    return estimate;
  }
  async additionalFees(gasLimit: number, block: Block): Promise<{ l1: bigint; operator: bigint }> {
    if (this.network.chain_id === 1) return { l1: 0n, operator: 0n };
    const [l1, operator] = await Promise.all([
      this.read("eth_call", [{ to: ORACLE, data: `${L1_UPPER}${(1024n).toString(16).padStart(64, "0")}` }, this.tag(block)]),
      this.read("eth_call", [{ to: ORACLE, data: `${OPERATOR}${BigInt(gasLimit).toString(16).padStart(64, "0")}` }, this.tag(block)]),
    ]);
    return { l1: abiUint(l1), operator: abiUint(operator) };
  }
  private afford(snapshot: BalanceSnapshot, asset: Asset, amount: bigint, reserve: bigint): void {
    if (decimal(snapshot.balances[asset]) < amount) throw new Error(`Insufficient ${asset} on ${this.network.network}.`);
    const nativeNeeded = reserve + (asset === "ETH" ? amount : 0n);
    if (decimal(snapshot.balances.ETH) < nativeNeeded) throw new Error(`More ETH is required on ${this.network.network} for the reviewed network fee reserve.`);
  }
  async quote(intent: string, from: string, to: string, asset: Asset, amount: string): Promise<Quote> {
    address(from); address(to);
    const amountUnits = units(amount, networkAsset(this.network, asset).decimals);
    if (amountUnits === 0n) throw new Error("Enter an amount greater than zero.");
    await this.verifyChain();
    const block = await this.block("latest", true);
    const tx = rpcTransaction(this.network, from, to, asset, amountUnits);
    const [nonce, priorityRaw, gasEstimate, snapshot] = await Promise.all([
      this.nonce(from), this.read("eth_maxPriorityFeePerGas"), this.simulate(tx, asset), this.balanceAt(from, block),
      this.noDelegation(from, block), this.tokenChecks(asset, block),
    ]);
    const priority = quantity(priorityRaw);
    const maxFee = quantity(block.baseFeePerGas) * 2n + priority;
    const padded = (gasEstimate * 125n + 99n) / 100n;
    const floor = minimumGas(tx.data);
    const gasLimit = padded > floor ? padded : floor;
    if (gasLimit > 300_000n) throw new Error("This transfer exceeds the wallet’s gas limit after adding its safety margin.");
    const extra = await this.additionalFees(Number(gasLimit), block);
    const reserve = gasLimit * maxFee + 2n * (extra.l1 + extra.operator);
    this.afford(snapshot, asset, amountUnits, reserve);
    await this.assertCanonical(block);
    return {
      request: {
        intent_id: intent, chain_id: this.network.chain_id, from, to, asset, amount, nonce,
        gas_limit: Number(gasLimit), max_fee_per_gas: maxFee.toString(), max_priority_fee_per_gas: priority.toString(),
        l1_fee_estimate_wei: extra.l1.toString(), operator_fee_estimate_wei: extra.operator.toString(),
      }, snapshot,
    };
  }
  async revalidate(prepared: PreparedPayment): Promise<void> {
    const p = prepared.request;
    if (p.chain_id !== this.network.chain_id) throw new Error("Wrong network for this review.");
    if (prepared.expires_at <= Math.floor(Date.now() / 1000)) throw new Error("This review expired. Discard the draft and prepare a fresh review.");
    await this.verifyChain();
    const block = await this.block("latest", true);
    const tx = rpcTransaction(this.network, p.from, p.to, p.asset, decimal(prepared.amount_units));
    tx.maxFeePerGas = hexQuantity(decimal(p.max_fee_per_gas));
    tx.maxPriorityFeePerGas = hexQuantity(decimal(p.max_priority_fee_per_gas));
    tx.gas = hexQuantity(p.gas_limit);
    const [nonce, gas, snapshot, extra] = await Promise.all([
      this.nonce(p.from), this.simulate(tx, p.asset), this.balanceAt(p.from, block), this.additionalFees(p.gas_limit, block),
      this.noDelegation(p.from, block), this.tokenChecks(p.asset, block),
    ]);
    if (nonce !== p.nonce) throw new Error("The account nonce changed. Discard this unsigned review and reconcile before trying again.");
    if (gas > BigInt(p.gas_limit) || quantity(block.baseFeePerGas) + decimal(p.max_priority_fee_per_gas) > decimal(p.max_fee_per_gas)) throw new Error("Network gas conditions changed beyond the reviewed limits. Create a new review.");
    if (decimal(prepared.max_fee_wei) + extra.l1 + extra.operator > decimal(prepared.fee_reserve_wei)) throw new Error("Base’s additional fees exceeded the reviewed reserve. Create a new review.");
    this.afford(snapshot, p.asset, decimal(prepared.amount_units), decimal(prepared.fee_reserve_wei));
    await this.assertCanonical(block);
  }
  async broadcast(signed: SignedPayment): Promise<void> {
    if (signed.chain_id !== this.network.chain_id) throw new Error("Wrong network for the signed payment.");
    // Only the exact durably stored transaction may reach this method.
    const returned = await this.transport.broadcast(signed.raw_transaction);
    if (hash(returned) !== hash(signed.hash)) throw new RpcError("The provider returned a different transaction hash. Submission outcome is unknown.");
  }
  async receipt(signed: SignedPayment): Promise<Record<string, unknown> | null> {
    const result = await this.read("eth_getTransactionReceipt", [signed.hash]);
    if (result === null) return null;
    if (typeof result !== "object" || Array.isArray(result)) throw new Error("Invalid transaction receipt.");
    const receipt = result as Record<string, unknown>;
    if (hash(receipt.transactionHash) !== hash(signed.hash)) throw new Error("The provider returned a receipt for another transaction.");
    hash(receipt.blockHash); quantity(receipt.blockNumber);
    return receipt;
  }
  async receiptFees(receipt: Record<string, unknown>): Promise<Record<string, unknown>> {
    if (this.network.chain_id !== 8453) return receipt;
    // Never infer a missing L1 charge as zero. It is part of the receipt.
    quantity(receipt.l1Fee);
    const gas = quantity(receipt.gasUsed);
    const operator = abiUint(await this.read("eth_call", [
      { to: ORACLE, data: `${OPERATOR}${gas.toString(16).padStart(64, "0")}` },
      { blockHash: hash(receipt.blockHash), requireCanonical: true },
    ]));
    if (receipt.operatorFee !== undefined && quantity(receipt.operatorFee) !== operator) throw new Error("The Base receipt disagrees with the operator fee at its block.");
    return { ...receipt, operatorFee: hexQuantity(operator) };
  }
  async transactionPresent(signed: SignedPayment): Promise<boolean> {
    const value = await this.read("eth_getTransactionByHash", [signed.hash]) as Record<string, unknown> | null;
    if (value === null) return false;
    if (hash(value.hash) !== hash(signed.hash) || !sameAddress(address(value.from), signed.from) || safeNumber(quantity(value.nonce)) !== signed.nonce || (value.chainId !== undefined && quantity(value.chainId) !== BigInt(signed.chain_id))) throw new Error("The provider returned conflicting transaction metadata.");
    return true;
  }
}
