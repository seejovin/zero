package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigDecimal
import java.math.BigInteger

class WithdrawalFlowTest {
    private val networks = TestNetworks.all
    private val sender = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    private val recipient = "0x1234567890123456789012345678901234567890"

    private fun request(symbol: String = "USDC", chain: Long? = 8453L, amount: String = "1.000001",
                        to: String = recipient, mode: WithdrawalAmountMode = WithdrawalAmountMode.MANUAL) =
        WithdrawalRequest.create(networks, sender, chain, symbol, to, amount, mode)

    private fun fee(request: WithdrawalRequest, amount: String = request.amount ?: "1.000001",
                    funded: Boolean = true, tokensAvailable: Boolean = true, simulated: Boolean = true): FeePreview {
        val units = Amounts.parse(amount, request.position.asset.decimals)
        val l1 = if (request.network.feeProfile == NetworkFeeProfile.OP_STACK) BigInteger("123") else BigInteger.ZERO
        val operator = if (request.network.feeProfile == NetworkFeeProfile.OP_STACK) BigInteger("10000") else BigInteger.ZERO
        val executionLimit = BigInteger("240000000000")
        val reserve = executionLimit + (l1 + operator) * BigInteger.valueOf(2)
        val needed = reserve + if (request.symbol == "ETH") units else BigInteger.ZERO
        val eth = if (funded) needed + BigInteger.ONE else BigInteger.ZERO
        val assetAvailable = if (request.symbol == "ETH") eth else if (tokensAvailable) units else units - BigInteger.ONE
        return FeePreview(request.network, request.symbol, request.recipient, amount, 30_000,
            BigInteger("120000000000"), executionLimit, l1, operator, reserve, eth, needed,
            if (request.symbol == "ETH") eth >= units else tokensAvailable, simulated,
            if (simulated) "Live estimate" else "Conservative budget", 1_000_000L, assetAvailable)
    }

    private fun quoted(request: WithdrawalRequest, fee: FeePreview = fee(request),
                       maximum: MaxTransferQuote? = null): Pair<WithdrawalFlow, WithdrawalQuote> {
        val flow = WithdrawalFlow()
        flow.update(request)
        val ticket = flow.beginQuote(1_000L)!!
        assertTrue(flow.complete(ticket, fee, maximum, 1_100L))
        return flow to flow.currentQuote(1_100L)!!
    }

    private fun maximum(request: WithdrawalRequest, fee: FeePreview, balance: BigInteger? = null,
                        observedAt: Long = 1_000L): MaxTransferQuote {
        val units = Amounts.parse(fee.amount, request.position.asset.decimals)
        return MaxTransferQuote(request.from, request.network, request.symbol, request.recipient, fee.amount,
            units, balance ?: if (request.symbol == "ETH") units + fee.reserve else units,
            fee, 100L, "0x" + "a".repeat(64), observedAt)
    }

    @Test fun inputsRequireAnExplicitSupportedNetworkAndPreserveEveryNativeUnit() {
        assertThrows(WalletFailure::class.java) { request(chain = null) }
        assertThrows(WalletFailure::class.java) { request(chain = 84532L) }
        assertThrows(WalletFailure::class.java) { request(symbol = "USDT") }
        assertEquals("0.000001", request(amount = " 0.000001 ").amount)
        assertEquals("0.000000000000000001", request(symbol = "ETH", amount = "0.000000000000000001").amount)
        assertEquals("1.23", request(amount = "1.230000").amount)
        for (invalid in listOf("", "0", "1e-6", "1,000", "+1", "-1", "1.0000001")) {
            assertThrows(WalletFailure::class.java) { request(amount = invalid) }
        }
        val tooLarge = BigInteger.ONE.shiftLeft(256).toString()
        assertThrows(WalletFailure::class.java) { request(amount = tooLarge) }
        val max = request(mode = WithdrawalAmountMode.MAX, amount = "")
        assertNull(max.amount)
        assertEquals(WithdrawalAmountMode.MAX, max.mode)
    }

    @Test fun newNetworkQuotesRetainTheirExactChainAndRejectFeesFromAnotherFeeModel() {
        val op = request(chain = 10L, amount = "0.000001")
        val (_, opQuote) = quoted(op)
        assertEquals(10L, opQuote.request.network.chainId)
        assertEquals(BigInteger.ONE, opQuote.recipientUnits)
        assertEquals(BigInteger("10123"), opQuote.fee.l1Estimate + opQuote.fee.operatorEstimate)
        val arb = request(chain = 42161L, amount = "0.000001")
        val (_, arbQuote) = quoted(arb)
        assertEquals(42161L, arbQuote.request.network.chainId)
        assertEquals(arbQuote.fee.executionLimit, arbQuote.fee.reserve)
        val bad = fee(arb).let { it.copy(l1Estimate = BigInteger.ONE, reserve = it.reserve + BigInteger.TWO,
            ethNeeded = it.ethNeeded + BigInteger.TWO) }
        val flow = WithdrawalFlow().apply { update(arb) }
        val ticket = flow.beginQuote(1_000L)!!
        assertThrows(WalletFailure::class.java) { flow.complete(ticket, bad, null, 1_100L) }
        assertNull(flow.claimPreparation(1_100L))
        for (chain in listOf(10L, 42161L)) assertThrows(WalletFailure::class.java) { request(chain = chain, symbol = "EURC") }
    }

    @Test fun pastedRecipientIsTrimmedWithoutRemovingItsChecksumAndKnownTokenContractsAreRejected() {
        val mixedCase = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        assertEquals(mixedCase, request(to = " \n$mixedCase\t").recipient)
        assertEquals(mixedCase, request(to = " $mixedCase ", mode = WithdrawalAmountMode.MAX).recipient)
        for (invalid in listOf("", "0x" + "0".repeat(40), "alice.eth", "ethereum:$recipient@8453", "0x1234")) {
            assertThrows(WalletFailure::class.java) { request(to = invalid) }
        }
        for (contract in networks.flatMap { it.assets }.mapNotNull { it.contract }) {
            assertThrows(WalletFailure::class.java) { request(to = contract) }
            assertThrows(WalletFailure::class.java) { request(to = contract.lowercase(), mode = WithdrawalAmountMode.MAX) }
        }
    }

    @Test fun oldNewOldDraftSequenceAndReplayedRepliesCannotOverwriteTheCurrentQuote() {
        val flow = WithdrawalFlow()
        val a = request()
        val b = request(chain = 1L, amount = "2")
        flow.update(a)
        val originalA = flow.beginQuote(1_000L)!!
        flow.update(b)
        val oldB = flow.beginQuote(1_010L)!!
        flow.update(a)
        val currentA = flow.beginQuote(1_020L)!!
        assertFalse(flow.complete(originalA, fee(a), null, 1_100L))
        assertFalse(flow.fail(oldB))
        assertNull(flow.currentQuote(1_100L))
        assertTrue(flow.complete(currentA, fee(a), null, 1_100L))
        val accepted = flow.currentQuote(1_100L)!!
        assertFalse(flow.complete(currentA, fee(a).copy(amount = "2"), null, 1_110L))
        assertFalse(flow.fail(originalA))
        assertSame(accepted, flow.currentQuote(1_110L))
    }

    @Test fun pageInvalidationAndAnotherFlowCannotReviveAnOutstandingQuote() {
        val a = request()
        val first = WithdrawalFlow().apply { update(a) }
        val firstTicket = first.beginQuote(1_000L)!!
        val second = WithdrawalFlow().apply { update(a) }
        val secondTicket = second.beginQuote(1_010L)!!
        assertFalse(second.complete(firstTicket, fee(a), null, 1_100L))
        first.invalidate()
        assertFalse(first.complete(firstTicket, fee(a), null, 1_100L))
        first.update(a)
        assertNull(first.beginQuote(1_100L))
        assertNull(first.claimPreparation(1_100L))
        assertTrue(second.complete(secondTicket, fee(a), null, 1_100L))
        assertNotNull(second.currentQuote(1_100L))
    }

    @Test fun failedOrMalformedCurrentResponsesLeaveNoFeeOrPreparationAuthority() {
        val a = request()
        val corrupt = listOf(
            fee(a).copy(network = TestNetworks.ethereum), fee(a).copy(symbol = "EURC"),
            fee(a).copy(recipient = sender), fee(a).copy(amount = "2"),
            fee(a).copy(executionEstimate = BigInteger.valueOf(-1)),
            fee(a).copy(executionEstimate = fee(a).executionLimit + BigInteger.ONE),
            fee(a).copy(reserve = fee(a).reserve + BigInteger.ONE),
            fee(a).copy(ethNeeded = fee(a).ethNeeded + BigInteger.ONE),
            fee(a).copy(ethAvailable = BigInteger.ONE.shiftLeft(256)),
            fee(a).copy(assetAvailable = null), fee(a).copy(assetAvailable = BigInteger.valueOf(-1)),
            fee(a).copy(assetAvailable = BigInteger.ONE.shiftLeft(256)),
            fee(a).copy(assetAvailable = BigInteger.ZERO),
            fee(a).copy(gasLimit = 300_001), fee(a).copy(executionLimit = BigInteger.ZERO)
        )
        for (bad in corrupt) {
            val flow = WithdrawalFlow().apply { update(a) }
            val ticket = flow.beginQuote(1_000L)!!
            assertThrows(WalletFailure::class.java) { flow.complete(ticket, bad, null, 1_100L) }
            assertNull(flow.currentQuote(1_100L))
            assertNull(flow.claimPreparation(1_100L))
            assertFalse(flow.complete(ticket, fee(a), null, 1_101L))
        }
        val failed = WithdrawalFlow().apply { update(a) }
        val ticket = failed.beginQuote(1_000L)!!
        assertTrue(failed.fail(ticket))
        assertNull(failed.currentQuote(1_100L))
        assertNull(failed.claimPreparation(1_100L))
        assertFalse(failed.complete(ticket, fee(a), null, 1_100L))
    }

    @Test fun quotesExpireFromMonotonicRequestStartAndRejectClockRollback() {
        val a = request()
        for (completedAt in listOf(999L, 31_000L, 60_000L)) {
            val flow = WithdrawalFlow().apply { update(a) }
            val ticket = flow.beginQuote(1_000L)!!
            assertThrows(WalletFailure::class.java) { flow.complete(ticket, fee(a).copy(observedAt = Long.MAX_VALUE), null, completedAt) }
            assertNull(flow.currentQuote(completedAt))
        }
        val (flow, quote) = quoted(a)
        assertEquals(29_900L, quote.remainingMillis(1_100L))
        assertNotNull(flow.currentQuote(30_999L))
        assertNull(flow.currentQuote(31_000L))
        assertNull(flow.claimPreparation(31_000L))
        assertNull(flow.currentQuote(999L))
    }

    @Test fun tokenFeesRemainSeparateFromRecipientUnitsAndUseOnlyAvailableUsdQuotes() {
        val a = request(amount = "0.000001")
        val (_, quote) = quoted(a)
        assertEquals(BigInteger.ONE, quote.recipientUnits)
        assertEquals("0.000001", quote.amount)
        assertEquals(quote.fee.reserve, quote.fee.ethNeeded)
        assertTrue(quote.readyToPrepare)
        val prices = mapOf("USDC" to UsdPrices.quote("USDC", "0.9999", 1_000L),
            "ETH" to UsdPrices.quote("ETH", "3000", 1_000L))
        assertEquals(0, BigDecimal("0.0000009999").compareTo(quote.recipientUsd(prices, 1_100L).amount!!))
        assertEquals("<\$0.01", UsdPrices.formatUsd(quote.estimatedFeeUsd(prices, 1_100L).amount!!))
        assertEquals(PriceFreshness.UNAVAILABLE, quote.recipientUsd(emptyMap(), 1_100L).freshness)
        assertNull(quote.estimatedFeeUsd(emptyMap(), 1_100L).amount)
        assertEquals(PriceFreshness.STALE, quote.estimatedFeeUsd(prices, 122_000L).freshness)
        assertEquals("0.000001", quote.amount)
    }

    @Test fun tokenOrGasShortfallCannotEnableReviewAndConservativeBudgetDoesNotClaimSimulation() {
        val a = request()
        for (quotedFee in listOf(fee(a, funded = false), fee(a, tokensAvailable = false))) {
            val (flow, quote) = quoted(a, quotedFee)
            assertFalse(quote.readyToPrepare)
            assertNull(flow.claimPreparation(1_200L))
            assertEquals(a.amount, quote.amount)
        }
        val (flow, conservative) = quoted(a, fee(a, simulated = false))
        assertFalse(conservative.fee.simulated)
        assertEquals("Conservative budget", conservative.fee.note)
        // A conservative funded budget permits only fresh preparation; it never authorizes signing.
        assertNotNull(flow.claimPreparation(1_200L))
    }

    @Test fun availableAmountComesFromTheSameFreshObservationThatGatesReview() {
        val a = request(amount = "0.01")
        val (flow, prior) = quoted(a, fee(a).copy(assetAvailable = BigInteger("100000000")))
        assertEquals(BigInteger("100000000"), prior.assetAvailable)
        val newTicket = flow.beginQuote(1_200L)!!
        assertNull(flow.currentQuote(1_201L))
        val decreased = fee(a).copy(assetAvailable = BigInteger("9999"), transferAvailable = false)
        assertTrue(flow.complete(newTicket, decreased, null, 1_300L))
        val current = flow.currentQuote(1_300L)!!
        assertEquals(BigInteger("9999"), current.assetAvailable)
        assertEquals("0.01", current.amount)
        assertFalse(current.readyToPrepare)
        assertNull(flow.claimPreparation(1_300L))

        val eth = request(symbol = "ETH", amount = "1")
        val ethFlow = WithdrawalFlow().apply { update(eth) }
        val ticket = ethFlow.beginQuote(1_000L)!!
        val inconsistent = fee(eth).copy(assetAvailable = fee(eth).ethAvailable - BigInteger.ONE)
        assertThrows(WalletFailure::class.java) { ethFlow.complete(ticket, inconsistent, null, 1_100L) }
        assertNull(ethFlow.claimPreparation(1_100L))
    }

    @Test fun maxUsesEveryTokenUnitAndExactlyOneEthReserveWithoutDecimalTruncation() {
        for ((symbol, amount) in listOf("USDC" to "123.456789", "EURC" to "0.000001", "ETH" to "0.123456789012345678")) {
            val a = request(symbol = symbol, mode = WithdrawalAmountMode.MAX, amount = "")
            var f = fee(a, amount)
            val balance = Amounts.parse(amount, a.position.asset.decimals) + if (symbol == "ETH") f.reserve else BigInteger.ZERO
            if (symbol == "ETH") f = f.copy(ethAvailable = balance, assetAvailable = balance)
            val max = maximum(a, f, balance)
            val (flow, quote) = quoted(a, f, max)
            assertEquals(amount, quote.amount)
            assertEquals(Amounts.parse(amount, a.position.asset.decimals), quote.recipientUnits)
            assertSame(max, flow.claimPreparation(1_200L)!!.maximum)
            if (symbol == "ETH") assertEquals(balance, quote.recipientUnits + quote.fee.reserve)
            else assertEquals(balance, quote.recipientUnits)
        }
    }

    @Test fun maxRequiresMatchingFreshEvidenceAndCannotBeInjectedIntoAManualAmount() {
        val a = request(mode = WithdrawalAmountMode.MAX)
        val f = fee(a)
        val valid = maximum(a, f)
        val incorrect = listOf(
            valid.copy(from = recipient), valid.copy(network = TestNetworks.ethereum), valid.copy(symbol = "EURC"),
            valid.copy(recipient = sender), valid.copy(amount = "2"), valid.copy(units = valid.units - BigInteger.ONE),
            valid.copy(balanceUnits = valid.units + BigInteger.ONE), valid.copy(observedAtElapsed = 999L),
            valid.copy(fee = f.copy(observedAt = 99L)), null
        )
        for (bad in incorrect) {
            val flow = WithdrawalFlow().apply { update(a) }
            val ticket = flow.beginQuote(1_000L)!!
            assertThrows(WalletFailure::class.java) { flow.complete(ticket, f, bad, 1_100L) }
            assertNull(flow.claimPreparation(1_100L))
        }
        val manual = request()
        val flow = WithdrawalFlow().apply { update(manual) }
        val ticket = flow.beginQuote(1_000L)!!
        assertThrows(WalletFailure::class.java) { flow.complete(ticket, f, valid, 1_100L) }
    }

    @Test fun oneCurrentObservationCanBeClaimedOnlyOnceAndEditsInvalidateItsPreparationResult() {
        val a = request()
        val (flow, original) = quoted(a)
        val claim = flow.claimPreparation(1_200L)!!
        assertSame(original, claim)
        assertTrue(flow.preparationCurrent(claim))
        assertNull(flow.claimPreparation(1_201L))
        assertNull(flow.beginQuote(1_201L))
        flow.update(request(amount = "2"))
        assertFalse(flow.preparationCurrent(claim))
        assertNull(flow.claimPreparation(1_202L))
        assertNull(flow.beginQuote(1_202L))
        flow.update(a)
        assertFalse(flow.preparationCurrent(claim))
        flow.releasePreparation()
        assertNull(flow.claimPreparation(1_203L))
        val fresh = flow.beginQuote(1_204L)!!
        assertTrue(flow.complete(fresh, fee(a), null, 1_205L))
        assertNotSame(claim, flow.claimPreparation(1_206L))
        flow.invalidate()
        assertFalse(flow.preparationCurrent(claim))
    }
}
