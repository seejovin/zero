package app.localwallet.mainnet

import org.json.JSONArray
import org.json.JSONObject
import java.math.BigInteger

/** Read-only cost calculator; cannot access a vault, reserve a nonce, sign, or broadcast. */
object FeeEstimator {
    /** BSC's London envelope has a zero base fee. Quote its actual gas price explicitly. */
    internal fun baseFee(network: WalletNetwork, block: JSONObject): BigInteger {
        val base = Amounts.quantity(block.getString("baseFeePerGas"))
        if (network.feeProfile == NetworkFeeProfile.BNB && base != BigInteger.ZERO) {
            throw WalletFailure("BNB fee rules changed. This wallet cannot safely quote this transfer.")
        }
        return base
    }

    internal fun priorityFee(network: WalletNetwork, rpc: NetworkClient): BigInteger =
        Amounts.quantity(rpc.string(if (network.feeProfile == NetworkFeeProfile.BNB) "eth_gasPrice" else "eth_maxPriorityFeePerGas", JSONArray()))

    internal fun validateTokenResult(network: WalletNetwork, asset: AssetInfo, result: String) {
        if (asset.contract == null) return
        if (NetworkPolicy.permitsEmptyTransferResult(network.chainId, asset.symbol) &&
            asset.contract.equals("0xdAC17F958D2ee523a2206206994597C13D831ec7", ignoreCase = true) && result == "0x") return
        if (Amounts.abiInteger(result) != BigInteger.ONE) throw WalletFailure("Token transfer simulation was rejected.")
    }

    fun estimate(network: WalletNetwork, from: String, symbol: String, recipient: String, amount: String,
                 block: JSONObject, values: Map<String, BigInteger>, rpc: NetworkClient,
                 requireSimulation: Boolean = false, priorityFeePerGas: BigInteger? = null,
                 nonce: Long? = null, includeFeeFields: Boolean = false): FeePreview {
        Amounts.address(recipient)
        val asset = network.asset(symbol)
        val units = Amounts.parse(amount, asset.decimals)
        val base = baseFee(network, block)
        val priority = priorityFeePerGas ?: priorityFee(network, rpc)
        val maximum = base.multiply(BigInteger.valueOf(2)).add(priority)
        // These chain-specific ceilings are repeated by the Rust signer.
        if (requireSimulation && (priority < BigInteger.ZERO || maximum <= BigInteger.ZERO || maximum > NetworkPolicy.maximumFeePerGas(network.chainId))) {
            throw WalletFailure("Network fees are outside this wallet's transfer policy. Max is unavailable.")
        }
        val tx = JSONObject().put("from", from).put("to", asset.contract ?: recipient)
            .put("value", if (asset.contract == null) Amounts.hex(units) else "0x0")
            .put("data", if (asset.contract == null) "0x" else Amounts.transferData(recipient, units))
        if (nonce != null) tx.put("nonce", Amounts.hex(BigInteger.valueOf(nonce)))
        if (includeFeeFields) tx.put("maxFeePerGas", Amounts.hex(maximum)).put("maxPriorityFeePerGas", Amounts.hex(priority))
        var simulated = false
        var note = if (network.feeProfile == NetworkFeeProfile.ARBITRUM)
            "Live estimate includes execution and L1 data posting. Fees are paid in ${network.nativeSymbol} on ${network.name}."
        else "Live estimate. Network fees are paid in ${network.nativeSymbol} on ${network.name}."
        val gasEstimate = try {
            // Ordinary token quotes omit fee fields so lack of ETH does not hide the fee shortfall.
            // Native Max and final preparation include them, matching the proposed transaction.
            val result = rpc.string("eth_call", JSONArray().put(tx).put(rpc.reference(block)))
            validateTokenResult(network, asset, result)
            val estimate = Amounts.longQuantity(rpc.string("eth_estimateGas", JSONArray().put(tx).put("pending")))
            if (estimate !in 21_000L..300_000L) throw WalletFailure("Gas estimate is outside transfer policy.")
            simulated = true
            estimate
        } catch (error: Exception) {
            if (requireSimulation) throw error
            // A plain ETH transfer on Arbitrum still pays L1 posting gas; never invent
            // a 21,000-gas total when the live estimate is unavailable.
            val ordinaryNative = network.feeProfile != NetworkFeeProfile.ARBITRUM && asset.contract == null &&
                rpc.string("eth_getCode", JSONArray().put(recipient).put(rpc.reference(block))) == "0x"
            note = if (ordinaryNative) "Budget for a direct ${network.nativeSymbol} transfer. A fresh simulation is required before sending."
                else "Conservative budget using this wallet's 300,000-gas ceiling; simulation is not yet available. An exact check is required before sending."
            if (ordinaryNative) 21_000L else 300_000L
        }
        val paddedGas = (gasEstimate * 120 + 99) / 100
        if (requireSimulation && paddedGas > 300_000) {
            throw WalletFailure("Gas estimate plus safety margin exceeds this wallet's policy. Max is unavailable.")
        }
        val gasLimit = if (simulated) paddedGas.coerceAtMost(300_000) else gasEstimate
        val l1 = rpc.l1Estimate(rpc.reference(block))
        val operator = rpc.operatorEstimate(gasLimit, rpc.reference(block))
        val executionLimit = maximum.multiply(BigInteger.valueOf(gasLimit))
        val reserve = executionLimit + (l1 + operator).multiply(BigInteger.valueOf(2))
        if (requireSimulation && reserve > NetworkPolicy.maximumFeeReserve(network.chainId)) {
            throw WalletFailure("The fee reserve exceeds this wallet's transfer policy. Max is unavailable.")
        }
        val needed = reserve + if (asset.contract == null) units else BigInteger.ZERO
        return FeePreview(network, symbol, recipient, amount, gasLimit,
            (base + priority).multiply(BigInteger.valueOf(gasEstimate)), executionLimit, l1, operator,
            reserve, values.getValue(network.nativeSymbol), needed, values.getValue(symbol) >= units,
            simulated, note, System.currentTimeMillis(), values.getValue(symbol))
    }
}
