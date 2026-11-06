package app.localwallet.mainnet

import org.json.JSONException
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import uniffi.wallet_core.WalletException
import uniffi.wallet_core.WalletJournal
import uniffi.wallet_core.preparePayment
import java.io.File
import java.math.BigInteger
import java.nio.file.Files
import javax.crypto.Cipher

/** Actual compiled Kotlin repository/UniFFI/Rust SQLite; public synthetic records, no network or key use. */
class WalletRepositoryStorageTest {
    private val publicAddress = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

    private class AppDirectory : AutoCloseable {
        val root = Files.createTempDirectory("wallet-parent-alias-")
        val physical = root.resolve("data/data/app.localwallet.mainnet/no_backup/wallet-v1").also { Files.createDirectories(it) }
        val alias = root.resolve("data/user/0/app.localwallet.mainnet/no_backup/wallet-v1")
        val sentinel = "public fixture: existing vault bytes must remain unchanged".toByteArray()
        init {
            Files.createDirectories(root.resolve("data/user"))
            Files.createSymbolicLink(root.resolve("data/user/0"), root.resolve("data/data"))
            Files.write(physical.resolve("vault.json"), sentinel)
        }
        override fun close() { root.toFile().deleteRecursively() }
    }

    private fun repository(directory: File, address: String = publicAddress): WalletRepository {
        val file = object : VaultFile {
            override val directory = directory
            override fun exists(): Boolean = error("Journal operations must not open the encrypted vault")
            override fun read(): ByteArray = error("Journal operations must not read keys")
            override fun write(value: ByteArray) = error("Journal operations must not replace the vault")
        }
        val keys = object : VaultKeys {
            override fun create(mode: AppLockMode): VaultEnrollment = error("No key creation in storage tests")
            override fun decrypt(manifest: VaultManifest): Cipher = error("No signing or secret access in storage tests")
            override fun delete(alias: String) = error("No key deletion in storage tests")
        }
        val vault = SecureVault(file, keys, { 1_000_000L }) { error("No vault decoding in storage tests") }
        // Journal operations only use the public account identity, never these unused encrypted-key fields.
        val manifest = VaultManifest(address, "unused", "unused", ByteArray(12), ByteArray(48), true)
        return WalletRepository(vault, manifest)
    }

    private fun snapshot(block: Long = 100, units: String = "10000"): JSONObject = JSONObject()
        .put("chain_id", 8453).put("address", publicAddress).put("block_number", block)
        .put("block_hash", "0x" + (if (block == 100L) "a" else "b").repeat(64))
        .put("balances", JSONObject().put("ETH", "0").put("USDC", units).put("EURC", "0"))

    private fun prepared(): String = preparePayment(JSONObject()
        .put("intent_id", "public-storage-fixture").put("chain_id", 8453).put("from", publicAddress)
        .put("to", "0x1111111111111111111111111111111111111111").put("asset", "USDC").put("amount", "0.01")
        .put("nonce", 0).put("gas_limit", 100000).put("max_fee_per_gas", "1000000000")
        .put("max_priority_fee_per_gas", "100000000").put("l1_fee_estimate_wei", "1234")
        .put("operator_fee_estimate_wei", "5678").toString())

    @Test fun frameworkParentAliasReopensTheSameDurableBalancesAndHistory() {
        AppDirectory().use { directory ->
            val originalPath = directory.alias.resolve("journal-8453.sqlite3")
            // Reproduce the former 0.5.0 path against the unchanged native library.
            assertThrows(WalletException.Storage::class.java) { WalletJournal(originalPath.toString()).close() }
            var preparedRecord = ""
            repository(directory.alias.toFile()).use { repo ->
                val base = repo.networks.first { it.chainId == 8453L }
                assertNull(repo.cachedBalance(base))
                assertTrue(repo.history(base).isEmpty())
                val observed = repo.saveBalanceObservation(base, snapshot(), 1_000_000)
                assertEquals(BigInteger("10000"), observed.balances["USDC"])
                assertEquals(1_000_000L, observed.observedAt)
                WalletJournal(journalFile(directory.alias.toFile(), 8453L).absolutePath).use { journal ->
                    preparedRecord = journal.recordPrepared(prepared())
                }
                assertEquals("public-storage-fixture", repo.history(base).single().getString("intent_id"))
                val ethereum = repo.networks.first { it.chainId == 1L }
                assertNull(repo.cachedBalance(ethereum))
                assertTrue(repo.history(ethereum).isEmpty())
            }
            assertTrue(Files.isSameFile(originalPath, directory.physical.resolve("journal-8453.sqlite3")))
            repository(directory.alias.toFile()).use { reopened ->
                val base = reopened.networks.first { it.chainId == 8453L }
                assertEquals(BigInteger("10000"), reopened.cachedBalance(base)!!.balances["USDC"])
                assertEquals(0L, reopened.cachedBalance(base)!!.observedAt)
                assertEquals(JSONObject(preparedRecord).toString(), reopened.history(base).single().toString())
                reopened.saveBalanceObservation(base, snapshot(101, "20000"), 2_000_000)
                assertEquals(JSONObject(preparedRecord).toString(), reopened.history(base).single().toString())
            }
            repository(directory.alias.toFile()).use { reopened ->
                val base = reopened.networks.first { it.chainId == 8453L }
                assertEquals(BigInteger("20000"), reopened.cachedBalance(base)!!.balances["USDC"])
                assertEquals("public-storage-fixture", reopened.history(base).single().getString("intent_id"))
            }
            assertArrayEquals(directory.sentinel, Files.readAllBytes(directory.physical.resolve("vault.json")))
            assertTrue(Files.isSymbolicLink(directory.root.resolve("data/user/0")))
        }
    }

    @Test fun newNetworkJournalsShareAnExistingWalletWithoutReplacingItsVaultOrBaseRecords() {
        AppDirectory().use { directory ->
            var baseHistory = ""
            repository(directory.alias.toFile()).use { repo ->
                val base = repo.networks.first { it.chainId == 8453L }
                repo.saveBalanceObservation(base, snapshot(), 1_000_000)
                WalletJournal(journalFile(directory.alias.toFile(), 8453L).absolutePath).use { it.recordPrepared(prepared()) }
                baseHistory = repo.history(base).single().toString()
                for (chain in listOf(10L, 42161L)) {
                    val network = repo.networks.first { it.chainId == chain }
                    assertNull(repo.cachedBalance(network))
                    assertTrue(repo.history(network).isEmpty())
                    val observation = snapshot().put("chain_id", chain).apply { getJSONObject("balances").remove("EURC") }
                    val saved = repo.saveBalanceObservation(network, observation, 2_000_000)
                    assertEquals(setOf("ETH", "USDC"), saved.balances.keys)
                    assertEquals(BigInteger("10000"), saved.balances.getValue("USDC"))
                    val unsupported = JSONObject(observation.toString()).put("block_number", 101)
                        .put("block_hash", "0x" + "b".repeat(64)).apply { getJSONObject("balances").put("EURC", "0") }
                    assertThrows(LocalRecordFailure::class.java) { repo.saveBalanceObservation(network, unsupported, 3_000_000) }
                    assertEquals(100L, repo.cachedBalance(network)!!.block)
                }
                assertEquals(baseHistory, repo.history(base).single().toString())
                assertEquals(BigInteger("10000"), repo.cachedBalance(base)!!.balances.getValue("USDC"))
            }
            repository(directory.alias.toFile()).use { reopened ->
                for (chain in listOf(10L, 42161L)) {
                    val network = reopened.networks.first { it.chainId == chain }
                    val balance = reopened.cachedBalance(network)!!
                    assertEquals(setOf("ETH", "USDC"), balance.balances.keys)
                    assertEquals(BigInteger("10000"), balance.balances.getValue("USDC"))
                    assertEquals(0L, balance.observedAt)
                    assertTrue(Files.exists(directory.physical.resolve("journal-$chain.sqlite3")))
                }
                assertEquals(baseHistory, reopened.history(reopened.networks.first { it.chainId == 8453L }).single().toString())
            }
            assertArrayEquals(directory.sentinel, Files.readAllBytes(directory.physical.resolve("vault.json")))
        }
    }

    @Test fun databaseLeafSymlinkIsRejectedWithoutModifyingItsTarget() {
        AppDirectory().use { directory ->
            val target = directory.root.resolve("untouched-database-target")
            val existing = "existing target must not be opened or replaced".toByteArray()
            Files.write(target, existing)
            val leaf = directory.physical.resolve("journal-8453.sqlite3")
            Files.createSymbolicLink(leaf, target)
            repository(directory.alias.toFile()).use { repo ->
                val failure = assertThrows(LocalRecordFailure::class.java) { repo.cachedBalance(repo.networks.first { it.chainId == 8453L }) }
                assertEquals(LocalRecordStage.JOURNAL_OPEN, failure.stage)
            }
            assertTrue(Files.isSymbolicLink(leaf))
            assertArrayEquals(existing, Files.readAllBytes(target))
            assertArrayEquals(directory.sentinel, Files.readAllBytes(directory.physical.resolve("vault.json")))
        }
    }

    @Test fun appControlledWalletDirectorySymlinkIsRejectedBeforeOpeningAnyJournal() {
        AppDirectory().use { directory ->
            val target = directory.physical.resolveSibling("original-wallet")
            Files.move(directory.physical, target)
            Files.createSymbolicLink(directory.physical, target)
            repository(directory.alias.toFile()).use { repo ->
                val failure = assertThrows(LocalRecordFailure::class.java) { repo.cachedBalance(repo.networks.first { it.chainId == 8453L }) }
                assertEquals(LocalRecordStage.JOURNAL_PATH, failure.stage)
            }
            assertFalse(Files.exists(target.resolve("journal-8453.sqlite3")))
            assertArrayEquals(directory.sentinel, Files.readAllBytes(target.resolve("vault.json")))
            assertTrue(Files.isSymbolicLink(directory.physical))
        }
    }

    @Test fun unreadableExistingJournalIsPreservedAndNeverReplacedWithAnEmptyOne() {
        AppDirectory().use { directory ->
            val database = directory.physical.resolve("journal-8453.sqlite3")
            val original = "invalid existing journal must not be reset".toByteArray()
            Files.write(database, original)
            repository(directory.alias.toFile()).use { repo ->
                val failure = assertThrows(LocalRecordFailure::class.java) { repo.cachedBalance(repo.networks.first { it.chainId == 8453L }) }
                assertEquals(LocalRecordStage.JOURNAL_OPEN, failure.stage)
            }
            assertArrayEquals(original, Files.readAllBytes(database))
            assertArrayEquals(directory.sentinel, Files.readAllBytes(directory.physical.resolve("vault.json")))
        }
    }

    @Test fun rejectedObservationDoesNotEraseLastBalanceOrExistingHistory() {
        AppDirectory().use { directory ->
            repository(directory.alias.toFile()).use { repo ->
                val base = repo.networks.first { it.chainId == 8453L }
                repo.saveBalanceObservation(base, snapshot(), 1_000_000)
                WalletJournal(journalFile(directory.alias.toFile(), 8453L).absolutePath).use { it.recordPrepared(prepared()) }
                val invalid = snapshot(101, "-1")
                val failure = assertThrows(LocalRecordFailure::class.java) { repo.saveBalanceObservation(base, invalid, 2_000_000) }
                assertEquals(LocalRecordStage.BALANCE_SAVE, failure.stage)
                assertEquals(BigInteger("10000"), repo.cachedBalance(base)!!.balances["USDC"])
                assertEquals("public-storage-fixture", repo.history(base).single().getString("intent_id"))
            }
            repository(directory.alias.toFile()).use { repo ->
                assertEquals(BigInteger("10000"), repo.cachedBalance(repo.networks.first { it.chainId == 8453L })!!.balances["USDC"])
            }
        }
    }

    @Test fun refreshDiagnosticsDistinguishLocalRecordsFromRpcAndDoNotExposeRawErrors() {
        val sensitiveDetail = "/private/fixture/path never display raw record contents"
        val local = WalletDiagnostics.refreshMessage("Base", WalletException.Storage(sensitiveDetail))
        assertTrue(local.contains("LOCAL_JOURNAL_IO"))
        assertFalse(local.contains("Check your connection"))
        assertFalse(local.contains(sensitiveDetail))
        val saved = WalletDiagnostics.refreshMessage("Base", LocalRecordFailure(LocalRecordStage.BALANCE_SAVE))
        assertTrue(saved.contains("Network balances were checked"))
        assertTrue(saved.contains("LOCAL_BALANCE_SAVE"))
        val network = WalletDiagnostics.refreshMessage("Base", NetworkReadUnavailable("Base"))
        assertTrue(network.contains("RPC_UNAVAILABLE"))
        assertFalse(network.contains("LOCAL_"))
        val malformed = WalletDiagnostics.refreshMessage("Base", JSONException(sensitiveDetail))
        assertTrue(malformed.contains("BALANCE_VERIFY"))
        assertFalse(malformed.contains(sensitiveDetail))
    }

    @Test fun wrongAccountNetworkAndOlderObservationsCannotReplaceTheLastBalance() {
        AppDirectory().use { directory ->
            repository(directory.alias.toFile()).use { repo ->
                val base = repo.networks.first { it.chainId == 8453L }
                repo.saveBalanceObservation(base, snapshot(), 1_000_000)
                val invalid = listOf(
                    snapshot(101, "20000").put("address", "0x1111111111111111111111111111111111111111"),
                    snapshot(101, "20000").put("chain_id", 1),
                    snapshot(101, "20000").put("chain_id", 84532),
                    snapshot(99, "20000"),
                    snapshot(100, "20000")
                )
                invalid.forEach { observation ->
                    val failure = assertThrows(LocalRecordFailure::class.java) { repo.saveBalanceObservation(base, observation, 2_000_000) }
                    assertEquals(LocalRecordStage.BALANCE_SAVE, failure.stage)
                    assertEquals(100L, repo.cachedBalance(base)!!.block)
                    assertEquals(BigInteger("10000"), repo.cachedBalance(base)!!.balances["USDC"])
                }
            }
        }
    }

    @Test fun savedRecordsForAnotherAccountOrNetworkRemainBlockedOnRead() {
        AppDirectory().use { directory ->
            repository(directory.alias.toFile()).use { repo ->
                repo.saveBalanceObservation(repo.networks.first { it.chainId == 8453L }, snapshot(), 1_000_000)
                WalletJournal(journalFile(directory.alias.toFile(), 8453L).absolutePath).use { it.recordPrepared(prepared()) }
            }
            repository(directory.alias.toFile(), "0x1111111111111111111111111111111111111111").use { other ->
                val base = other.networks.first { it.chainId == 8453L }
                assertEquals(LocalRecordStage.BALANCE_READ, assertThrows(LocalRecordFailure::class.java) { other.cachedBalance(base) }.stage)
                assertEquals(LocalRecordStage.HISTORY_READ, assertThrows(LocalRecordFailure::class.java) { other.history(base) }.stage)
            }
            Files.copy(directory.physical.resolve("journal-8453.sqlite3"), directory.physical.resolve("journal-1.sqlite3"))
            repository(directory.alias.toFile()).use { repo ->
                val ethereum = repo.networks.first { it.chainId == 1L }
                assertEquals(LocalRecordStage.BALANCE_READ, assertThrows(LocalRecordFailure::class.java) { repo.cachedBalance(ethereum) }.stage)
                assertEquals(LocalRecordStage.HISTORY_READ, assertThrows(LocalRecordFailure::class.java) { repo.history(ethereum) }.stage)
                assertEquals(BigInteger("10000"), repo.cachedBalance(repo.networks.first { it.chainId == 8453L })!!.balances["USDC"])
            }
        }
    }
    @Test fun newNativeBalanceJournalsUseSeparatePathsAndLegacyEthereumUsdtStaysUnknown() {
        AppDirectory().use { directory ->
            repository(directory.alias.toFile()).use { repo ->
                for (network in repo.networks.filter { it.chainId in setOf(43114L, 137L, 56L) }) {
                    val observation = snapshot().put("chain_id", network.chainId).put("balances",
                        JSONObject(network.assets.associate { it.symbol to if (it.contract == null) "123456789123456789" else "10000" }))
                    val saved = repo.saveBalanceObservation(network, observation, 2000L)
                    assertEquals(BigInteger("123456789123456789"), saved.balances[network.nativeSymbol])
                    assertFalse("ETH" in saved.balances)
                    assertTrue(Files.exists(directory.physical.resolve("journal-${network.chainId}.sqlite3")))
                    val wrong = JSONObject(observation.toString()).apply { getJSONObject("balances").put("ETH", "0") }
                    assertThrows(LocalRecordFailure::class.java) { repo.saveBalanceObservation(network, wrong, 3000L) }
                }
                val ethereum = repo.networks.first { it.chainId == 1L }
                val old = snapshot().put("chain_id", 1L)
                val restored = repo.saveBalanceObservation(ethereum, old, 0L)
                assertEquals(setOf("ETH", "USDC", "EURC"), restored.balances.keys)
                assertNull(restored.balances["USDT"])
                assertEquals(BigInteger("10000"), restored.balances["USDC"])
            }
            assertArrayEquals(directory.sentinel, Files.readAllBytes(directory.physical.resolve("vault.json")))
        }
    }

}
