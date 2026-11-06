package app.localwallet.mainnet

import java.math.BigInteger

/** Reviewed fee semantics, never inferred from a token ticker or arbitrary RPC response. */
enum class NetworkFeeProfile {
    ETHEREUM,
    /** Execution gas plus separately charged L1 data and operator fees. */
    OP_STACK,
    /** Gas estimation already includes L1 posting costs. */
    ARBITRUM,
    AVALANCHE,
    POLYGON,
    /** London-compatible type-2 envelope, zero base fee; explicit gas-price quotation. */
    BNB
}

/** Fixed mainnet allowlist. A chain requires matching signer policy and regression coverage. */
object NetworkPolicy {
    val evmChainOrder: List<Long> = listOf(8453L, 1L, 10L, 42161L, 43114L, 137L, 56L)
    val chainOrder: List<Long> = evmChainOrder + listOf(FamilyNetworks.SOLANA, FamilyNetworks.TRON)
    val assetSymbols = listOf("USDC", "EURC", "USDT", "ETH", "SOL", "TRX", "AVAX", "POL", "BNB")

    fun family(chainId: Long): ChainFamily = when (chainId) {
        FamilyNetworks.SOLANA -> ChainFamily.SOLANA
        FamilyNetworks.TRON -> ChainFamily.TRON
        in evmChainOrder -> ChainFamily.EVM
        else -> unsupported()
    }

    fun name(chainId: Long): String = when (chainId) {
        1L -> "Ethereum"
        8453L -> "Base"
        10L -> "OP Mainnet"
        42161L -> "Arbitrum One"
        43114L -> "Avalanche C-Chain"
        137L -> "Polygon PoS"
        56L -> "BNB Smart Chain"
        FamilyNetworks.SOLANA -> "Solana"
        FamilyNetworks.TRON -> "TRON"
        else -> unsupported()
    }

    fun nativeSymbol(chainId: Long): String = when (chainId) {
        1L, 8453L, 10L, 42161L -> "ETH"
        43114L -> "AVAX"
        137L -> "POL"
        56L -> "BNB"
        FamilyNetworks.SOLANA -> "SOL"
        FamilyNetworks.TRON -> "TRX"
        else -> unsupported()
    }

    fun feeProfile(chainId: Long): NetworkFeeProfile = when (chainId) {
        1L -> NetworkFeeProfile.ETHEREUM
        8453L, 10L -> NetworkFeeProfile.OP_STACK
        42161L -> NetworkFeeProfile.ARBITRUM
        43114L -> NetworkFeeProfile.AVALANCHE
        137L -> NetworkFeeProfile.POLYGON
        56L -> NetworkFeeProfile.BNB
        else -> unsupported()
    }

    fun glyph(chainId: Long): String = when (chainId) {
        1L -> "ETH"
        8453L -> "BASE"
        10L -> "OP"
        42161L -> "ARB"
        43114L -> "AVAX"
        137L -> "POL"
        56L -> "BNB"
        FamilyNetworks.SOLANA -> "SOL"
        FamilyNetworks.TRON -> "TRX"
        else -> unsupported()
    }

    fun supports(chainId: Long, symbol: String): Boolean {
        if (chainId == FamilyNetworks.SOLANA) return symbol in setOf("SOL", "USDC", "EURC", "USDT")
        if (chainId == FamilyNetworks.TRON) return symbol in setOf("TRX", "USDT")
        if (chainId !in evmChainOrder) return false
        return symbol == nativeSymbol(chainId) || when (symbol) {
            "USDC" -> chainId != 56L
            "EURC" -> chainId in setOf(1L, 8453L, 43114L)
            "USDT" -> chainId in setOf(1L, 43114L)
            else -> false
        }
    }

    fun decimals(chainId: Long, symbol: String): Int {
        if (!supports(chainId, symbol)) unsupported()
        return if (symbol != nativeSymbol(chainId)) 6 else when (chainId) {
            FamilyNetworks.SOLANA -> 9
            FamilyNetworks.TRON -> 6
            else -> 18
        }
    }

    /** Local spending ceilings, denominated in each chain's actual gas currency. */
    fun maximumFeePerGas(chainId: Long): BigInteger = BigInteger(when (chainId) {
        1L, 8453L, 10L, 42161L, 43114L -> "500000000000"
        137L -> "5000000000000"
        56L -> "10000000000"
        else -> unsupported()
    })

    fun maximumFeeReserve(chainId: Long): BigInteger = BigInteger(when (chainId) {
        1L, 8453L, 10L, 42161L -> "50000000000000000" // 0.05 ETH
        43114L -> "500000000000000000" // 0.5 AVAX
        137L -> "1000000000000000000" // 1 POL
        56L -> "10000000000000000" // 0.01 BNB
        else -> unsupported()
    })

    /** The older Ethereum USDT transfer ABI deliberately returns no value. */
    fun permitsEmptyTransferResult(chainId: Long, symbol: String): Boolean = chainId == 1L && symbol == "USDT"

    private fun unsupported(): Nothing = throw WalletFailure("Unsupported network.")
}
