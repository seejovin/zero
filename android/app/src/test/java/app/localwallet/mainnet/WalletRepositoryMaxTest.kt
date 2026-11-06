package app.localwallet.mainnet

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import java.io.File
import java.math.BigInteger
import java.nio.file.Files
import javax.crypto.Cipher

/** Actual Kotlin repository and Rust preparation/journal, synthetic public reads, no real keys or network. */
class WalletRepositoryMaxTest {
    private val sender = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    private val recipient = "0x1111111111111111111111111111111111111111"

    private class Reads {
        var eth = BigInteger("1000000000000000001")
        var token = BigInteger("123456789")
        var baseFee = BigInteger("5000000")
        var elapsed = 1_000L
        var stale = false
        var reorganized = false
        var expireDuringRead = false
        var changeBalanceAfterSimulation = false
        var rejectSimulation = false
        val methods = mutableListOf<String>()
        fun block() = JSONObject().put("number", "0x64").put("hash", "0x" + "a".repeat(64))
            .put("timestamp", Amounts.hex(BigInteger.valueOf(System.currentTimeMillis() / 1000 - if (stale) 61 else 0)))
            .put("baseFeePerGas", Amounts.hex(baseFee))
        fun client(network: WalletNetwork) = NetworkClient(network, RpcTransport { _, method, params ->
            methods.add(method)
            when (method) {
                "eth_chainId" -> Amounts.hex(BigInteger.valueOf(network.chainId))
                "eth_getBlockByNumber" -> block().also {
                    if (reorganized && params.getString(0) != "latest") it.put("hash", "0x" + "b".repeat(64))
                }
                "eth_getBalance" -> Amounts.hex(eth)
                "eth_getCode" -> if (network.assets.any { it.contract?.equals(params.getString(0), true) == true }) "0x6000" else "0x"
                "eth_getTransactionCount" -> "0x0"
                "eth_maxPriorityFeePerGas", "eth_gasPrice" -> "0xf4240"
                "eth_estimateGas" -> {
                    if (expireDuringRead) elapsed += 30_000L
                    if (changeBalanceAfterSimulation) eth += BigInteger.ONE
                    "0x5208"
                }
                "eth_call" -> {
                    val tx = params.getJSONObject(0)
                    val data = tx.getString("data")
                    when {
                        data.startsWith("0x70a08231") -> "0x" + Amounts.word(token)
                        data.startsWith(NetworkClient.L1_UPPER_BOUND_SELECTOR) -> "0x" + Amounts.word(BigInteger("123"))
                        data.startsWith(NetworkClient.OPERATOR_FEE_SELECTOR) -> "0x" + Amounts.word(BigInteger("10000"))
                        rejectSimulation -> throw RpcRejected(RpcRejectionKind.REVERTED, 3)
                        data == "0x" -> "0x"
                        else -> "0x" + Amounts.word(BigInteger.ONE)
                    }
                }
                else -> throw AssertionError("Unexpected Max operation: $method")
            }
        })
    }

    @Before fun reset() = NetworkClient.clearPreferenceForTests()

    private fun repository(directory: File, reads: Reads, backupConfirmed: Boolean = false): WalletRepository {
        val file = object : VaultFile {
            override val directory = directory
            override fun exists(): Boolean = error("Max must not open the vault")
            override fun read(): ByteArray = error("Max must not read encrypted keys")
            override fun write(value: ByteArray) = error("Max must not change the vault")
        }
        val keys = object : VaultKeys {
            override fun create(mode: AppLockMode): VaultEnrollment = error("No key creation")
            override fun decrypt(manifest: VaultManifest): Cipher = error("No key access")
            override fun delete(alias: String) = error("No key deletion")
        }
        val vault = SecureVault(file, keys, { 1_000_000L }) { error("No vault decoding") }
        val manifest = VaultManifest(sender, "unused", "unused", ByteArray(12), ByteArray(48), backupConfirmed)
        return WalletRepository(vault, manifest, reads::client) { reads.elapsed }
    }

    private fun fixture(action: (File, Reads, WalletRepository) -> Unit) {
        val directory = Files.createTempDirectory("wallet-max-public-fixture-").toFile()
        try {
            val reads = Reads()
            repository(directory, reads).use { action(directory, reads, it) }
        } finally { directory.deleteRecursively() }
    }

    @Test fun quotingMaxReadsLiveBalancesWithoutJournalNonceKeyOrSubmissionSideEffects() = fixture { directory, reads, repo ->
        val base = repo.networks.first { it.chainId == 8453L }
        val token = repo.maxTransfer(base, "USDC", recipient)
        assertEquals("123.456789", token.amount)
        val eth = repo.maxTransfer(base, "ETH", recipient)
        assertEquals("0.999999722799979755", eth.amount)
        assertTrue(directory.listFiles()!!.isEmpty())
        assertFalse(reads.methods.any { it == "eth_getTransactionCount" || it.contains("send", true) || it.contains("sign", true) })
        assertFalse(repo.manifest.backupConfirmed)
    }

    @Test fun slowStaleOrReorganizedQuoteDoesNotCreateAnIntentOrReturnACachedMaximum() {
        for (failure in listOf("slow", "stale", "reorganization")) fixture { directory, reads, repo ->
            when (failure) {
                "slow" -> reads.expireDuringRead = true
                "stale" -> reads.stale = true
                "reorganization" -> reads.reorganized = true
            }
            assertThrows(WalletFailure::class.java) { repo.maxTransfer(repo.networks.first { it.chainId == 8453L }, "USDC", recipient) }
            assertTrue(directory.listFiles()!!.isEmpty())
        }
    }

    @Test fun finalTokenPreparationRefreshesTheEntireExactBalanceBeforeRecordingReview() = fixture { _, reads, repo ->
        val base = repo.networks.first { it.chainId == 8453L }
        val quote = repo.maxTransfer(base, "USDC", recipient)
        reads.token = BigInteger("123456791")
        val review = repo.prepare(base, "USDC", recipient, quote.amount, quote)
        assertEquals("123.456791", review.request.getString("amount"))
        assertEquals(reads.token, Amounts.parse(review.request.getString("amount"), 6))
        assertEquals("prepared", repo.history(base).single().getString("phase"))
        assertFalse(repo.manifest.backupConfirmed)
        assertFalse(reads.methods.any { it.contains("send", true) || it.contains("sign", true) })
    }

    @Test fun finalEthPreparationRefreshesAmountAndFeeTogetherAndThenFixesTheReviewedAmount() = fixture { _, reads, repo ->
        val base = repo.networks.first { it.chainId == 8453L }
        val quote = repo.maxTransfer(base, "ETH", recipient)
        reads.eth += BigInteger("789")
        reads.baseFee = BigInteger("6000000")
        val review = repo.prepare(base, "ETH", recipient, quote.amount, quote)
        val reviewedAmount = review.request.getString("amount")
        val reserve = BigInteger(review.objectValue.getString("fee_reserve_wei"))
        assertEquals(BigInteger("327600020246"), reserve)
        assertEquals(reads.eth, Amounts.parse(reviewedAmount, 18) + reserve)
        assertNotEquals(quote.amount, reviewedAmount)
        reads.eth += BigInteger.ONE
        reads.baseFee += BigInteger.ONE
        assertEquals(reviewedAmount, review.request.getString("amount"))
        assertEquals(reviewedAmount, repo.history(base).single().getString("amount"))
    }

    @Test fun editedOrExpiredMaxDraftIsRejectedBeforeAnyJournalIsOpened() {
        for (failure in listOf("amount", "recipient", "network", "asset", "expired")) fixture { directory, reads, repo ->
            val base = repo.networks.first { it.chainId == 8453L }
            val quote = repo.maxTransfer(base, "USDC", recipient)
            if (failure == "expired") reads.elapsed += 30_000L
            assertThrows(WalletFailure::class.java) {
                repo.prepare(if (failure == "network") repo.networks.first { it.chainId == 1L } else base,
                    if (failure == "asset") "EURC" else "USDC", if (failure == "recipient") sender else recipient,
                    if (failure == "amount") "123.45678" else quote.amount, quote)
            }
            assertTrue(directory.listFiles()!!.isEmpty())
        }
    }

    @Test fun failedRecomputationDoesNotLeaveAPreparedPayment() = fixture { _, reads, repo ->
        val base = repo.networks.first { it.chainId == 8453L }
        val quote = repo.maxTransfer(base, "USDC", recipient)
        reads.rejectSimulation = true
        assertThrows(RpcRejected::class.java) { repo.prepare(base, "USDC", recipient, quote.amount, quote) }
        assertTrue(repo.history(base).isEmpty())
    }

    @Test fun expirationOrInconsistentPinnedBalanceDuringPreparationCannotReserveAnIntent() {
        for (failure in listOf("expired", "inconsistent")) fixture { _, reads, repo ->
            val base = repo.networks.first { it.chainId == 8453L }
            val quote = repo.maxTransfer(base, "USDC", recipient)
            if (failure == "expired") reads.expireDuringRead = true else reads.changeBalanceAfterSimulation = true
            assertThrows(WalletFailure::class.java) { repo.prepare(base, "USDC", recipient, quote.amount, quote) }
            assertTrue(repo.history(base).isEmpty())
        }
    }

    @Test fun editingAReviewMustCancelItsExactUnsignedIntentBeforePreparingTheChangedTransfer() = fixture { _, reads, repo ->
        val base = repo.networks.first { it.chainId == 8453L }
        val original = repo.prepare(base, "USDC", recipient, "1.000001")
        assertThrows(WalletFailure::class.java) { repo.prepare(base, "USDC", recipient, "2.000002") }
        assertEquals(original.id, repo.history(base).single().getString("intent_id"))
        assertEquals("prepared", repo.history(base).single().getString("phase"))
        repo.cancelPrepared(base, original.id)
        val cancellationEvidence = repo.events(base, original.id)
        repo.cancelPrepared(base, original.id)
        assertEquals(cancellationEvidence, repo.events(base, original.id))
        val replacement = repo.prepare(base, "USDC", recipient, "2.000002")
        assertNotEquals(original.id, replacement.id)
        val records = repo.history(base)
        assertEquals("cancelled", records.single { it.getString("intent_id") == original.id }.getString("phase"))
        val active = records.single { it.getString("phase") == "prepared" }
        assertEquals(replacement.id, active.getString("intent_id"))
        assertEquals("2.000002", active.getString("amount"))
        assertEquals(original.request.getLong("nonce"), replacement.request.getLong("nonce"))
        assertFalse(reads.methods.any { it.contains("send", true) || it.contains("sign", true) })
    }

    @Test fun cancellingAnUnsignedReviewOnAnotherNetworkCannotReleaseTheOriginalReservation() = fixture { _, _, repo ->
        val base = repo.networks.first { it.chainId == 8453L }
        val ethereum = repo.networks.first { it.chainId == 1L }
        val original = repo.prepare(base, "USDC", recipient, "1.000001")
        assertThrows(Exception::class.java) { repo.cancelPrepared(ethereum, original.id) }
        assertEquals("prepared", repo.history(base).single().getString("phase"))
        assertThrows(WalletFailure::class.java) { repo.prepare(base, "USDC", recipient, "2.000002") }
        repo.cancelPrepared(base, original.id)
        assertEquals("cancelled", repo.history(base).single().getString("phase"))
    }
    @Test fun remainingNativeMaxPreparationsReserveTheActualGasCurrencyWithoutRounding() {
        for (chain in listOf(43114L, 137L, 56L)) fixture { _, reads, repo ->
            val network = repo.networks.first { it.chainId == chain }
            if (chain == 56L) reads.baseFee = BigInteger.ZERO
            val quote = repo.maxTransfer(network, network.nativeSymbol, recipient)
            val reviewed = repo.prepare(network, network.nativeSymbol, recipient, quote.amount, quote)
            val units = Amounts.parse(reviewed.request.getString("amount"), network.nativeAsset.decimals)
            val reserve = BigInteger(reviewed.objectValue.getString("fee_reserve_wei"))
            assertEquals(reads.eth, units + reserve)
            assertEquals(network.nativeSymbol, reviewed.request.getString("asset"))
            assertEquals(network.nativeSymbol, repo.history(network).single().getString("asset"))
            assertFalse(reads.methods.any { it.contains("send", true) || it.contains("sign", true) })
            if (chain == 56L) {
                assertEquals(reviewed.request.getString("max_fee_per_gas"), reviewed.request.getString("max_priority_fee_per_gas"))
                assertTrue("eth_gasPrice" in reads.methods)
            }
        }
    }

}
