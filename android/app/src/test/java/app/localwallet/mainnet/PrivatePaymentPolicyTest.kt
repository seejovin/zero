package app.localwallet.mainnet

import org.junit.Assert.*
import org.junit.Test
import java.math.BigInteger

/** Arithmetic/routing/release-policy tests. They do not generate or verify a ZK proof. */
class PrivatePaymentPolicyTest {
    private val asset = PrivatePaymentAsset(42161, "0xaf88d065e77c8cc2239327c5edb3a432268e5831", "USDC", 6)
    private fun plan(values: List<String> = listOf("1000001", "1000002"), service: String = "2", protocol: String = "1",
                     complete: Boolean = true, all: Boolean = true, consolidate: Boolean = false) =
        PrivateSpendPlan(values.map(::BigInteger), BigInteger(service), BigInteger(protocol), complete, all, consolidate)

    @Test fun maximumSubtractsExactFeesWithoutDroppingAnySmallestUnit() {
        val selected = plan()
        assertEquals(BigInteger("2000000"), selected.maximumUnits())
        assertEquals(BigInteger.ZERO, selected.changeFor(selected.maximumUnits()))
        val oneUnit = plan(listOf("4"))
        assertEquals(BigInteger.ONE, oneUnit.maximumUnits())
        assertEquals("0.000001", Amounts.display(oneUnit.maximumUnits(), 6))
    }

    @Test fun maximumRequiresACompleteSpendableBalanceAndExplicitConsolidation() {
        assertEquals(PrivateFailure.SYNC_INCOMPLETE,
            assertThrows(PrivatePaymentFailure::class.java) { plan(complete = false).maximumUnits() }.failure)
        for (selected in listOf(plan(all = false), plan(consolidate = true))) {
            assertEquals(PrivateFailure.CONSOLIDATION_REQUIRED,
                assertThrows(PrivatePaymentFailure::class.java) { selected.maximumUnits() }.failure)
        }
        // A manual amount may use a subset; that subset must never be labelled "Max".
        assertEquals(BigInteger("1999999"), plan(all = false).changeFor(BigInteger.ONE))
    }

    @Test fun feeEqualityOrDeficitNeverProducesAZeroOrNegativeMaximum() {
        for (balance in listOf("1", "2", "3")) {
            assertEquals(PrivateFailure.INSUFFICIENT_FUNDS,
                assertThrows(PrivatePaymentFailure::class.java) { plan(listOf(balance)).maximumUnits() }.failure)
        }
        assertThrows(PrivatePaymentFailure::class.java) { plan().changeFor(BigInteger("2000001")) }
    }

    @Test fun moreNotesThanTheCandidateCircuitLimitCannotSilentlyTruncateMaximum() {
        val selected = plan(List(17) { "10" }, protocol = "0")
        assertEquals(PrivateFailure.CONSOLIDATION_REQUIRED,
            assertThrows(PrivatePaymentFailure::class.java) { selected.maximumUnits() }.failure)
        assertThrows(PrivatePaymentFailure::class.java) { selected.changeFor(BigInteger.ONE) }
    }

    @Test fun integerParsingRejectsAmbiguityAndCheckedAdditionRejectsOverflow() {
        for (value in listOf("", "00", "01", "+1", "-1", "1.0", "1e6", " 1", "١")) {
            assertThrows(PrivatePaymentFailure::class.java) { PrivateUnits.fromCanonical(value) }
        }
        assertEquals(BigInteger.ZERO, PrivateUnits.fromCanonical("0"))
        val max = BigInteger.ONE.shiftLeft(256) - BigInteger.ONE
        assertEquals(max, PrivateUnits.fromCanonical(max.toString()))
        assertThrows(PrivatePaymentFailure::class.java) { PrivateUnits.add(max, BigInteger.ONE) }
        assertThrows(PrivatePaymentFailure::class.java) { plan(listOf(max.toString(), "1")) }
    }

    @Test fun spendPlanCannotBeMutatedByChangingItsInputList() {
        val source = mutableListOf(BigInteger("10"))
        val selected = PrivateSpendPlan(source, BigInteger.ONE, BigInteger.ZERO, true, true, false)
        source[0] = BigInteger("999")
        assertEquals(BigInteger("9"), selected.maximumUnits())
        assertFalse(selected.toString().contains("10"))
    }

    @Test fun privateDestinationsNeverEnterPublicEvmOrFamilyRecipientPaths() {
        for (destination in listOf("0zk12345678", " RAILGUN:example ", "private:example", "aztec:example")) {
            assertEquals(PrivateFailure.PRIVATE_DESTINATION_IN_PUBLIC_FLOW,
                assertThrows(PrivatePaymentFailure::class.java) { PrivateAddressRules.requirePublicEvmRecipient(destination) }.failure)
            assertThrows(PrivatePaymentFailure::class.java) { PrivateAddressRules.requirePublicText(destination) }
            assertThrows(PrivatePaymentFailure::class.java) {
                WithdrawalRequest.create(TestNetworks.all, "0x1111111111111111111111111111111111111111",
                    8453L, "USDC", destination, "0.000001")
            }
        }
        val publicAddress = "0x1234567890123456789012345678901234567890"
        assertEquals(publicAddress, PrivateAddressRules.requirePublicEvmRecipient(" $publicAddress "))
    }

    @Test fun privateDraftUsesRandomCanonicalIdentityAndNeverParsesMaximumThroughFloatingPoint() {
        val first = PrivateTransferDraft.create(asset, "0zkfixture-recipient", "0.000001")
        val second = PrivateTransferDraft.create(asset, "0zkfixture-recipient", "", maximum = true)
        assertEquals(BigInteger.ONE, first.recipientUnits)
        assertNull(second.recipientUnits)
        assertTrue(Regex("0x[0-9a-f]{64}").matches(first.intentId))
        assertNotEquals(first.intentId, second.intentId)
        assertFalse(first.toString().contains(first.recipient))
        for (recipient in listOf("0x1234567890123456789012345678901234567890", "0zkwith spaces", "0zkéexample", "0zk" + "a".repeat(254))) {
            assertThrows(PrivatePaymentFailure::class.java) { PrivateTransferDraft.create(asset, recipient, "1") }
        }
        assertThrows(PrivatePaymentFailure::class.java) { PrivateTransferDraft.create(asset, "0zkfixture-recipient", "0.0000001") }
    }

    @Test fun releaseHasNoEnabledNetworkOrUserSettingAndCannotOpenAProductionFeature() {
        val availability = PrivatePaymentRelease.availability()
        assertFalse(availability.available)
        assertTrue(availability.supportedChainIds.isEmpty())
        assertEquals(PrivateReleaseBlocker.entries.toSet(), availability.blockers)
        assertEquals(PrivateFailure.UNAVAILABLE,
            assertThrows(PrivatePaymentFailure::class.java) { PrivatePaymentRelease.requireAvailable() }.failure)
        assertThrows(PrivatePaymentFailure::class.java) { PrivateObliviousSubmissionTransport.create() }
    }
}
