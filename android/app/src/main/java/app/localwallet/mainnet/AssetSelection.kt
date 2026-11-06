package app.localwallet.mainnet

/** One canonical asset choice; its positions retain their distinct chain and token identities. */
data class AssetChoice(val symbol: String, val positions: List<PortfolioAsset>)

/**
 * Asset-first presentation for the existing reviewed mainnet registry only.
 * A ticker is never sufficient to admit an arbitrary token into one of these groups.
 */
object AssetSelection {
    private val assetOrder = NetworkPolicy.assetSymbols
    private val chainOrder = NetworkPolicy.chainOrder
    private val address = Regex("0x[0-9a-fA-F]{40}")
    // Deliberately pinned to core/src/crypto.rs. A bridge-registry test guards against drift.
    private val tokenContracts = mapOf(
        8453L to mapOf(
            "USDC" to "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "EURC" to "0x60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42"
        ),
        1L to mapOf(
            "USDC" to "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "EURC" to "0x1aBaEA1f7C830bD89Acc67eC4af516284b1bC33c",
            "USDT" to "0xdAC17F958D2ee523a2206206994597C13D831ec7"
        ),
        10L to mapOf("USDC" to "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85"),
        42161L to mapOf("USDC" to "0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
        43114L to mapOf(
            "USDC" to "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E",
            "EURC" to "0xC891EB4cbdEFf6e073e859e987815Ed1505c2ACD",
            "USDT" to "0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7"
        ),
        137L to mapOf("USDC" to "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359")
    )

    /** No balance, price, availability, or display-filter input can remove a supported choice. */
    fun assets(networks: List<WalletNetwork>): List<AssetChoice> {
        val checked = checkedNetworks(networks)
        return assetOrder.mapNotNull { symbol ->
            val positions = checked.mapNotNull { network ->
                network.assets.firstOrNull { it.symbol == symbol }?.let { PortfolioAsset(network, it) }
            }
            if (positions.isEmpty()) null else AssetChoice(symbol, positions)
        }
    }

    fun networksFor(networks: List<WalletNetwork>, symbol: String): List<WalletNetwork> =
        choice(networks, symbol).positions.map { it.network }

    /** Null stays unchosen; no remembered or first-listed network is silently substituted. */
    fun resolve(networks: List<WalletNetwork>, symbol: String, chainId: Long?): PortfolioAsset? {
        val selected = choice(networks, symbol)
        if (chainId == null) return null
        return selected.positions.firstOrNull { it.network.chainId == chainId }
            ?: throw WalletFailure("This asset is not supported on the selected network.")
    }

    fun requireSelected(networks: List<WalletNetwork>, symbol: String, chainId: Long?): PortfolioAsset =
        resolve(networks, symbol, chainId) ?: throw WalletFailure("Choose a network to continue.")

    private fun choice(networks: List<WalletNetwork>, symbol: String): AssetChoice =
        assets(networks).firstOrNull { it.symbol == symbol } ?: throw WalletFailure("Unsupported asset.")

    private fun checkedNetworks(networks: List<WalletNetwork>): List<WalletNetwork> {
        if (networks.isEmpty() || networks.size > chainOrder.size) invalidRegistry()
        val seenChains = HashSet<Long>()
        for (network in networks) {
            if (network.chainId !in chainOrder || !seenChains.add(network.chainId) || NetworkPolicy.name(network.chainId) != network.name ||
                network.assets.isEmpty() || network.assets.size > assetOrder.size) invalidRegistry()
            val seenAssets = HashSet<String>()
            for (asset in network.assets) {
                if (asset.symbol !in assetOrder || !seenAssets.add(asset.symbol) || !NetworkPolicy.supports(network.chainId, asset.symbol)) invalidRegistry()
                val expectedDecimals = NetworkPolicy.decimals(network.chainId, asset.symbol)
                if (asset.decimals != expectedDecimals) invalidRegistry()
                if (asset.symbol == network.nativeSymbol) {
                    if (asset.contract != null) invalidRegistry()
                } else {
                    val contract = asset.contract ?: invalidRegistry()
                    val family = NetworkPolicy.family(network.chainId)
                    val expected = if (family == ChainFamily.EVM) tokenContracts[network.chainId]?.get(asset.symbol)
                        else FamilyNetworks.token(network.chainId, asset.symbol)
                    if (expected == null) invalidRegistry()
                    if (family == ChainFamily.EVM) {
                        if (!address.matches(contract) || !contract.equals(expected, ignoreCase = true)) invalidRegistry()
                    } else if (contract != expected) invalidRegistry()
                }
            }
        }
        return networks.sortedBy { chainOrder.indexOf(it.chainId) }
    }

    private fun invalidRegistry(): Nothing = throw WalletFailure("The supported asset registry is invalid. No network has been selected.")
}
