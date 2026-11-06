package app.localwallet.mainnet

import java.math.BigInteger

object TestNetworks {
    val ethereum = WalletNetwork(1,"Ethereum","https://ethereum-rpc.publicnode.com","https://ethereum-rpc.publicnode.com","https://etherscan.io",
        listOf(AssetInfo("ETH",18,null),AssetInfo("USDC",6,"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),AssetInfo("EURC",6,"0x1aBaEA1f7C830bD89Acc67eC4af516284b1bC33c"),AssetInfo("USDT",6,"0xdAC17F958D2ee523a2206206994597C13D831ec7")))
    val base = WalletNetwork(8453,"Base","https://base-rpc.publicnode.com","https://mainnet-sequencer.base.org","https://basescan.org",
        listOf(AssetInfo("ETH",18,null),AssetInfo("USDC",6,"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),AssetInfo("EURC",6,"0x60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42")))
    val optimism = WalletNetwork(10,"OP Mainnet","https://optimism-rpc.publicnode.com","https://mainnet-sequencer.optimism.io","https://optimistic.etherscan.io",
        listOf(AssetInfo("ETH",18,null),AssetInfo("USDC",6,"0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85")))
    val arbitrum = WalletNetwork(42161,"Arbitrum One","https://arbitrum-one-rpc.publicnode.com","https://arb1-sequencer.arbitrum.io/rpc","https://arbiscan.io",
        listOf(AssetInfo("ETH",18,null),AssetInfo("USDC",6,"0xaf88d065e77c8cC2239327C5EDb3A432268e5831")))
    val avalanche = WalletNetwork(43114,"Avalanche C-Chain","https://api.avax.network/ext/bc/C/rpc","https://api.avax.network/ext/bc/C/rpc","https://snowtrace.io",
        listOf(AssetInfo("AVAX",18,null),AssetInfo("USDC",6,"0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E"),AssetInfo("EURC",6,"0xC891EB4cbdEFf6e073e859e987815Ed1505c2ACD"),AssetInfo("USDT",6,"0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7")))
    val polygon = WalletNetwork(137,"Polygon PoS","https://polygon-bor-rpc.publicnode.com","https://polygon-bor-rpc.publicnode.com","https://polygonscan.com",
        listOf(AssetInfo("POL",18,null),AssetInfo("USDC",6,"0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359")))
    val bnb = WalletNetwork(56,"BNB Smart Chain","https://bsc-dataseed.bnbchain.org","https://bsc-dataseed.bnbchain.org","https://bscscan.com",
        listOf(AssetInfo("BNB",18,null)))
    val evm = listOf(base, ethereum, optimism, arbitrum, avalanche, polygon, bnb)
    val all = evm + FamilyNetworks.all
    val live = all.associate { it.chainId to "live" }
    fun rollupZeros(observedAt: Long = 1000L) = all.filter { it.chainId !in setOf(1L, 8453L) }.associate { network -> network.chainId to
        BalanceView(100, "0x" + "a".repeat(64), network.assets.associate { it.symbol to BigInteger.ZERO }, observedAt) }
}
