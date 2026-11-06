package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import java.math.BigInteger

class MaxTransferTest {
    private val sender = "0x1111111111111111111111111111111111111111"
    private val recipient = "0x2222222222222222222222222222222222222222"
    private val balance = BigInteger("1000000000000000001")
    private val block = JSONObject().put("number", "0x64").put("hash", "0x" + "a".repeat(64)).put("baseFeePerGas", "0x4c4b40")
    private fun values(token: BigInteger = BigInteger("123456789"), eth: BigInteger = balance) =
        mapOf("ETH" to eth, "USDC" to token, "EURC" to token)

    @Before fun reset() = NetworkClient.clearPreferenceForTests()

    private fun rpc(network: WalletNetwork, estimates: MutableList<BigInteger> = mutableListOf(),
                    gas: (BigInteger) -> Long = { 21_000L },
                    failure: Exception? = null): NetworkClient = NetworkClient(network, RpcTransport { _, method, params ->
        when (method) {
            "eth_chainId" -> Amounts.hex(BigInteger.valueOf(network.chainId))
            "eth_maxPriorityFeePerGas" -> "0xf4240"
            "eth_estimateGas" -> {
                val tx = params.getJSONObject(0)
                val units = Amounts.quantity(tx.getString("value"))
                estimates.add(units)
                if (tx.getString("data") == "0x") {
                    assertEquals("0xa7d8c0", tx.getString("maxFeePerGas"))
                    assertEquals("0xf4240", tx.getString("maxPriorityFeePerGas"))
                }
                Amounts.hex(BigInteger.valueOf(gas(units)))
            }
            "eth_call" -> {
                val tx = params.getJSONObject(0)
                when {
                    tx.getString("data").startsWith(NetworkClient.L1_UPPER_BOUND_SELECTOR) -> {
                        assertEquals(BigInteger("1024"), BigInteger(tx.getString("data").substring(10), 16))
                        "0x" + Amounts.word(BigInteger("123"))
                    }
                    tx.getString("data").startsWith(NetworkClient.OPERATOR_FEE_SELECTOR) -> "0x" + Amounts.word(BigInteger("10000"))
                    failure != null -> throw failure
                    tx.getString("data") == "0x" -> "0x"
                    else -> "0x" + Amounts.word(BigInteger.ONE)
                }
            }
            else -> throw AssertionError("Max attempted unexpected operation: $method")
        }
    })

    @Test fun tokenMaxKeepsEveryUnitIncludingTheSmallestAndLargestQuantities() {
        for (network in TestNetworks.evm) {
            for (asset in network.assets.filter { it.contract != null }) {
                for (units in listOf(BigInteger.ONE, BigInteger("123456789"), BigInteger.ONE.shiftLeft(256).subtract(BigInteger.ONE))) {
                    val balances = network.assets.associate { it.symbol to if (it.contract == null) balance else units }
                    val result = MaxTransferCalculator.calculate(network, sender, asset.symbol, recipient, block, balances, rpc(network))
                    assertEquals(units, result.units)
                    assertEquals(units, Amounts.parse(result.amount, asset.decimals))
                    assertEquals(units, result.balanceUnits)
                    assertTrue(result.fee.simulated)
                }
            }
        }
        assertEquals("0.000001", Amounts.display(BigInteger.ONE, 6))
        assertEquals("0.000000000000000001", Amounts.display(BigInteger.ONE, 18))
        assertEquals("0.123456789123456789", Amounts.display(BigInteger("123456789123456789"), 18))
    }

    @Test fun baseEthMaxSubtractsExecutionL1AndOperatorReserveWithoutLosingAWei() {
        val estimates = mutableListOf<BigInteger>()
        val result = MaxTransferCalculator.calculate(TestNetworks.base, sender, "ETH", recipient, block, values(), rpc(TestNetworks.base, estimates))
        assertEquals(BigInteger("277200020246"), result.fee.reserve)
        assertEquals("0.999999722799979755", result.amount)
        assertEquals(balance, result.units + result.fee.reserve)
        assertEquals(balance, result.fee.ethNeeded)
        assertEquals(listOf(BigInteger.ONE, result.units), estimates)
        assertEquals(25_200L, result.fee.gasLimit)
        assertEquals(BigInteger.ZERO, result.fee.shortfall)
    }

    @Test fun tokenMaxNeverSubtractsEthFeesFromTokenUnitsAndReportsMissingGasFunds() {
        val result = MaxTransferCalculator.calculate(TestNetworks.base, sender, "USDC", recipient, block,
            values(token = BigInteger("10001"), eth = BigInteger.ZERO), rpc(TestNetworks.base))
        assertEquals("0.010001", result.amount)
        assertEquals(BigInteger("10001"), result.units)
        assertEquals(result.fee.reserve, result.fee.shortfall)
        assertTrue(result.fee.transferAvailable)
    }

    @Test fun ethereumEthMaxHasNoBaseFeesAndStillPreservesAllEighteenDecimals() {
        val result = MaxTransferCalculator.calculate(TestNetworks.ethereum, sender, "ETH", recipient, block, values(), rpc(TestNetworks.ethereum))
        assertEquals("0.999999722800000001", result.amount)
        assertEquals(BigInteger.ZERO, result.fee.l1Estimate)
        assertEquals(BigInteger.ZERO, result.fee.operatorEstimate)
        assertEquals(balance, result.units + result.fee.reserve)
    }

    @Test fun contractGasThatDependsOnAmountIsReestimatedForTheActualMaximum() {
        val estimates = mutableListOf<BigInteger>()
        val result = MaxTransferCalculator.calculate(TestNetworks.ethereum, sender, "ETH", recipient, block, values(),
            rpc(TestNetworks.ethereum, estimates, gas = { if (it > BigInteger("900000000000000000")) 50_000 else 21_000 }))
        assertEquals(3, estimates.size)
        assertEquals(result.units, estimates.last())
        assertEquals(60_000L, result.fee.gasLimit)
        assertEquals(BigInteger("660000000000"), result.fee.reserve)
        assertEquals(balance, result.units + result.fee.reserve)
    }

    @Test fun oscillatingContractGasDoesNotReturnAGuessedMaximum() {
        val estimates = mutableListOf<BigInteger>()
        val failure = assertThrows(WalletFailure::class.java) {
            MaxTransferCalculator.calculate(TestNetworks.ethereum, sender, "ETH", recipient, block, values(),
                rpc(TestNetworks.ethereum, estimates, gas = { if (it > BigInteger("999999500000000000")) 50_000 else 21_000 }))
        }
        assertTrue(failure.message!!.contains("gas requirement changed"))
        assertEquals(3, estimates.size)
    }

    @Test fun rejectedSimulationCannotBeReplacedWithTheOrdinaryPreviewBudgetFallback() {
        assertThrows(RpcRejected::class.java) {
            MaxTransferCalculator.calculate(TestNetworks.ethereum, sender, "USDC", recipient, block, values(),
                rpc(TestNetworks.ethereum, failure = RpcRejected(RpcRejectionKind.REVERTED, 3)))
        }
    }

    @Test fun missingNegativeZeroAndInsufficientBalancesAreRejected() {
        for (invalid in listOf(mapOf("ETH" to balance), values(BigInteger.valueOf(-1)), values(BigInteger.ZERO))) {
            assertThrows(WalletFailure::class.java) {
                MaxTransferCalculator.calculate(TestNetworks.base, sender, "USDC", recipient, block, invalid, rpc(TestNetworks.base))
            }
        }
        assertThrows(WalletFailure::class.java) {
            MaxTransferCalculator.calculate(TestNetworks.base, sender, "ETH", recipient, block, values(eth = BigInteger.ONE), rpc(TestNetworks.base))
        }
    }

    @Test fun maxNeverClampsTheRequiredGasMarginOrExceedsTheExistingFeePolicy() {
        assertThrows(WalletFailure::class.java) {
            MaxTransferCalculator.calculate(TestNetworks.ethereum, sender, "USDC", recipient, block, values(),
                rpc(TestNetworks.ethereum, gas = { 250_001 }))
        }
        val excessiveFees = JSONObject(block.toString()).put("baseFeePerGas", Amounts.hex(BigInteger("500000000000")))
        assertThrows(WalletFailure::class.java) {
            MaxTransferCalculator.calculate(TestNetworks.ethereum, sender, "USDC", recipient, excessiveFees, values(), rpc(TestNetworks.ethereum))
        }
    }

    @Test fun quoteBindingRejectsChangedWalletNetworkAssetRecipientAmountAndExpiredClock() {
        val result = MaxTransferCalculator.calculate(TestNetworks.base, sender, "USDC", recipient, block, values(), rpc(TestNetworks.base))
        val quote = MaxTransferQuote(sender, TestNetworks.base, "USDC", recipient, result.amount, result.units,
            result.balanceUnits, result.fee, 100L, "0x" + "a".repeat(64), 1_000L)
        assertTrue(quote.matches(sender, TestNetworks.base, "USDC", recipient, result.amount, 30_999L))
        assertFalse(quote.matches(sender, TestNetworks.base, "USDC", recipient, result.amount, 31_000L))
        assertFalse(quote.matches(sender, TestNetworks.base, "USDC", recipient, result.amount, 999L))
        assertFalse(quote.matches(recipient, TestNetworks.base, "USDC", recipient, result.amount, 1_000L))
        assertFalse(quote.matches(sender, TestNetworks.ethereum, "USDC", recipient, result.amount, 1_000L))
        assertFalse(quote.matches(sender, TestNetworks.base, "EURC", recipient, result.amount, 1_000L))
        assertFalse(quote.matches(sender, TestNetworks.base, "USDC", sender, result.amount, 1_000L))
        assertFalse(quote.matches(sender, TestNetworks.base, "USDC", recipient, "123.45678", 1_000L))
    }

    @Test fun anOldOrFutureBlockCannotBeCalledAFreshMaximum() {
        val snapshot = JSONObject().put("timestamp", Amounts.hex(BigInteger.valueOf(1_000)))
        MaxTransferCalculator.requireFreshBlock(snapshot, 1_060_000L)
        assertThrows(WalletFailure::class.java) { MaxTransferCalculator.requireFreshBlock(snapshot, 1_061_000L) }
        assertThrows(WalletFailure::class.java) { MaxTransferCalculator.requireFreshBlock(snapshot, 879_000L) }
    }
}
