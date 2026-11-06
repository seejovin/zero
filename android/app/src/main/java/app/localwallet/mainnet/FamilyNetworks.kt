package app.localwallet.mainnet

/** Local registry keys only. They never enter an EVM chain-id field. */
object FamilyNetworks {
    const val SOLANA = -101L
    const val TRON = -195L
    private val solanaTokens = mapOf(
        "USDC" to "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "EURC" to "HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr",
        "USDT" to "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"
    )
    private val tronTokens = mapOf("USDT" to "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")
    fun token(key: Long, symbol: String): String? = when (key) {
        SOLANA -> solanaTokens[symbol]
        TRON -> tronTokens[symbol]
        else -> null
    }
    val all: List<WalletNetwork> = listOf(
        WalletNetwork(SOLANA, "Solana", "https://api.mainnet-beta.solana.com", "https://api.mainnet-beta.solana.com",
            "https://explorer.solana.com", listOf(AssetInfo("SOL", 9, null)) + solanaTokens.map { AssetInfo(it.key, 6, it.value) }),
        WalletNetwork(TRON, "TRON", "https://api.trongrid.io", "https://api.trongrid.io",
            "https://tronscan.org/#", listOf(AssetInfo("TRX", 6, null)) + tronTokens.map { AssetInfo(it.key, 6, it.value) })
    )
    fun protocol(key: Long): String = when (key) {
        SOLANA -> "solana-mainnet"
        TRON -> "tron-mainnet"
        else -> throw WalletFailure("This network does not use the family transaction journal.")
    }
}
