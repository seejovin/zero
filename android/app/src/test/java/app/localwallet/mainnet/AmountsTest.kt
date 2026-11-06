package app.localwallet.mainnet

import java.math.BigInteger
import org.junit.Assert.*
import org.junit.Test

class AmountsTest {
    @Test fun sixDecimalAssetsKeepExactUnits() {
        assertEquals(BigInteger("1"), Amounts.parse("0.000001", 6))
        assertEquals(BigInteger("1234567890123456123456"), Amounts.parse("1234567890123456.123456", 6))
        assertEquals("1234567890123456.123456", Amounts.display(BigInteger("1234567890123456123456"), 6))
    }
    @Test fun etherKeepsWeiAndNeverRounds() {
        assertEquals(BigInteger.ONE, Amounts.parse("0.000000000000000001", 18))
        assertEquals("0.000000000000000001", Amounts.display(BigInteger.ONE, 18))
        assertEquals("1", Amounts.display(BigInteger.TEN.pow(18), 18))
    }
    @Test fun invalidAmountsAreNotNormalizedIntoPayments() {
        for (input in listOf("0", "0.0000000", "1.0000001", "-1", "+1", "1e6", "1,000", " 1", "1 ", ".1", "1.", "01", "1\n", "١", "NaN", "Infinity")) {
            try { Amounts.parse(input, 6); fail("Accepted invalid decimal") } catch (_: WalletFailure) { }
        }
    }
    @Test fun overflowCannotWrapIntoASmallTransfer() {
        val max = BigInteger.ONE.shiftLeft(256).subtract(BigInteger.ONE)
        assertEquals(max, Amounts.parse(max.toString(), 0))
        try { Amounts.parse(max.add(BigInteger.ONE).toString(), 0); fail("Accepted overflow") } catch (_: WalletFailure) { }
        try { Amounts.word(max.add(BigInteger.ONE)); fail("Encoded overflow") } catch (_: WalletFailure) { }
    }
    @Test fun rpcQuantitiesAreCanonicalAndBounded() {
        assertEquals(BigInteger.ZERO, Amounts.quantity("0x0"))
        assertEquals(BigInteger("8453"), Amounts.quantity("0x2105"))
        for (invalid in listOf("0x", "0x00", "0x01", "8453", "-0x1", "0X1", "0x1g", "0x" + "f".repeat(65))) {
            try { Amounts.quantity(invalid); fail("Accepted malformed quantity") } catch (_: WalletFailure) { }
        }
        try { Amounts.longQuantity("0x8000000000000000"); fail("Truncated nonce") } catch (_: WalletFailure) { }
    }
    @Test fun abiArgumentsKeepBeneficiaryAndAmountSeparate() {
        val recipient = "0x1234567890123456789012345678901234567890"
        val data = Amounts.transferData(recipient, BigInteger("1000001"))
        assertEquals("0xa9059cbb000000000000000000000000123456789012345678901234567890123456789000000000000000000000000000000000000000000000000000000000000f4241", data)
        assertEquals(BigInteger.ONE, Amounts.abiInteger("0x" + "0".repeat(63) + "1"))
        try { Amounts.abiInteger("0x1"); fail("Accepted truncated ABI word") } catch (_: WalletFailure) { }
    }
    @Test fun addressFieldsRejectUnicodeAndTruncation() {
        for (invalid in listOf("0x" + "0".repeat(40), "alice.eth", "0x1234", "0x" + "1".repeat(40) + "\n", "0x" + "1".repeat(39) + "\u202e")) {
            try { Amounts.address(invalid); fail("Accepted invalid address") } catch (_: WalletFailure) { }
        }
    }
}
