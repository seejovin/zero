package app.localwallet.mainnet

import org.json.JSONObject
import java.security.SecureRandom
import java.util.Base64
import java.util.UUID
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.PBEKeySpec
import javax.crypto.spec.SecretKeySpec

enum class AppLockMode(val stored: String, val label: String) {
    OFF("off", "Off"),
    PIN("pin", "6-digit app PIN"),
    DEVICE("device", "Biometrics or device screen lock");

    companion object {
        fun read(value: String): AppLockMode = entries.firstOrNull { it.stored == value }
            ?: throw WalletFailure("The saved app-lock method is unsupported. Existing wallet data has been preserved.")
    }
}

/** Version 1 always requires device authentication. Never infer Off from a missing field. */
data class VaultManifest(
    val address: String,
    val vaultHex: String,
    val alias: String,
    val iv: ByteArray,
    val wrappedKey: ByteArray,
    val backupConfirmed: Boolean,
    val version: Int = 1,
    val lockMode: AppLockMode = AppLockMode.DEVICE,
    val pinSalt: ByteArray? = null,
    val pinAttempts: Int = 0,
    val pinRetryAfter: Long = 0
) {
    fun json(): JSONObject = JSONObject().put("version", version).put("address", address)
        .put("vault_hex", vaultHex).put("key_alias", alias)
        .put("iv", Base64.getEncoder().encodeToString(iv))
        .put("wrapped_key", Base64.getEncoder().encodeToString(wrappedKey))
        .put("backup_confirmed", backupConfirmed).also {
            if (version == 2) it.put("lock_mode", lockMode.stored)
                .put("pin_salt", pinSalt?.let { salt -> Base64.getEncoder().encodeToString(salt) } ?: JSONObject.NULL)
                .put("pin_attempts", pinAttempts).put("pin_retry_after", pinRetryAfter)
        }

    fun aad(): ByteArray = if (version == 1) "LocalWalletAndroidWrap/v1|$alias|$address".toByteArray(Charsets.UTF_8)
        else "LocalWalletAndroidWrap/v2|$alias|$address|${lockMode.stored}|${pinSalt?.let { Base64.getEncoder().encodeToString(it) } ?: ""}".toByteArray(Charsets.UTF_8)

    fun sameProtection(other: VaultManifest): Boolean = version == other.version && address == other.address && vaultHex == other.vaultHex &&
        alias == other.alias && lockMode == other.lockMode && iv.contentEquals(other.iv) && wrappedKey.contentEquals(other.wrappedKey) &&
        (pinSalt == null && other.pinSalt == null || pinSalt != null && other.pinSalt != null && pinSalt.contentEquals(other.pinSalt))

    companion object {
        fun parse(value: JSONObject): VaultManifest {
            try {
                val version = exactLong(value, "version").also { if (it !in 1..2) throw WalletFailure("Unsupported wallet metadata version.") }.toInt()
                val expected = setOf("version", "address", "vault_hex", "key_alias", "iv", "wrapped_key", "backup_confirmed") +
                    if (version == 2) setOf("lock_mode", "pin_salt", "pin_attempts", "pin_retry_after") else emptySet()
                if (version !in 1..2 || value.keys().asSequence().toSet() != expected) throw WalletFailure("Unsupported wallet metadata. Existing wallet data has been preserved.")
                val address = Amounts.address(value.getString("address"))
                val vaultHex = value.getString("vault_hex")
                val alias = value.getString("key_alias")
                val prefix = "local-wallet-v$version-"
                if (!alias.startsWith(prefix) || UUID.fromString(alias.removePrefix(prefix)).toString() != alias.removePrefix(prefix) ||
                    !Regex("0x(?:[0-9a-fA-F]{2}){40,512}").matches(vaultHex)) {
                    throw WalletFailure("Wallet metadata is invalid. Existing data has not been replaced.")
                }
                val iv = Base64.getDecoder().decode(value.getString("iv"))
                val wrapped = Base64.getDecoder().decode(value.getString("wrapped_key"))
                val confirmed = value.get("backup_confirmed") as? Boolean ?: throw WalletFailure("Invalid backup confirmation state.")
                val mode = if (version == 1) AppLockMode.DEVICE else AppLockMode.read(value.getString("lock_mode"))
                val salt = if (version == 2 && !value.isNull("pin_salt")) Base64.getDecoder().decode(value.getString("pin_salt")) else null
                val attempts = if (version == 1) 0 else exactLong(value, "pin_attempts").also { if (it !in 0..1_000) throw WalletFailure("Invalid PIN retry record.") }.toInt()
                val retryAfter = if (version == 1) 0L else exactLong(value, "pin_retry_after").also { if (it < 0) throw WalletFailure("Invalid PIN retry record.") }
                if (iv.size != 12 || wrapped.size != if (mode == AppLockMode.PIN) 76 else 48) throw WalletFailure("Invalid encrypted wallet key.")
                if (mode == AppLockMode.PIN && salt?.size != 16 || mode != AppLockMode.PIN && (salt != null || attempts != 0 || retryAfter != 0L)) {
                    throw WalletFailure("Invalid app-lock metadata. Existing wallet data has been preserved.")
                }
                return VaultManifest(address, vaultHex, alias, iv, wrapped, confirmed, version, mode, salt, attempts, retryAfter)
            } catch (failure: WalletFailure) { throw failure }
            catch (_: Exception) { throw WalletFailure("Saved wallet metadata could not be read. Existing wallet data has been preserved; retry opening it.") }
        }

        private fun exactLong(value: JSONObject, key: String): Long {
            val field = value.get(key)
            if (field !is Number || !Regex("(?:0|[1-9][0-9]*)").matches(field.toString())) throw WalletFailure("Invalid PIN retry record.")
            return field.toString().toLong()
        }
    }
}

object VaultPin {
    private const val ITERATIONS = 600_000
    private val random = SecureRandom()
    fun requireSixDigits(pin: CharArray) {
        if (pin.size != 6 || pin.any { it !in '0'..'9' }) throw WalletFailure("Enter exactly 6 digits for your app PIN.")
    }

    /** The PIN ciphertext is itself wrapped by the non-exportable device key. */
    fun seal(key: ByteArray, pin: CharArray, salt: ByteArray, aad: ByteArray): ByteArray {
        requireSixDigits(pin)
        val derived = derive(pin, salt)
        try {
            val iv = ByteArray(12).also(random::nextBytes)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
                init(Cipher.ENCRYPT_MODE, SecretKeySpec(derived, "AES"), GCMParameterSpec(128, iv)); updateAAD(aad)
            }
            return iv + cipher.doFinal(key)
        } finally { derived.fill(0) }
    }

    fun open(value: ByteArray, pin: CharArray, salt: ByteArray, aad: ByteArray): ByteArray {
        requireSixDigits(pin)
        if (value.size != 60) throw WalletFailure("The encrypted PIN envelope is invalid.")
        val derived = derive(pin, salt)
        try {
            return Cipher.getInstance("AES/GCM/NoPadding").run {
                init(Cipher.DECRYPT_MODE, SecretKeySpec(derived, "AES"), GCMParameterSpec(128, value.copyOfRange(0, 12)))
                updateAAD(aad)
                doFinal(value, 12, value.size - 12)
            }
        } catch (_: AEADBadTagException) { throw WalletFailure("Incorrect 6-digit app PIN. Check your PIN and try again.") }
        finally { derived.fill(0) }
    }

    private fun derive(pin: CharArray, salt: ByteArray): ByteArray {
        if (salt.size != 16) throw WalletFailure("The PIN salt is invalid.")
        val spec = PBEKeySpec(pin, salt, ITERATIONS, 256)
        try { return SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256").generateSecret(spec).encoded }
        finally { spec.clearPassword() }
    }

    fun retryDelayMillis(attempts: Int): Long = if (attempts < 5) 0L else minOf(3_600_000L, 30_000L shl minOf(7, attempts - 5))
}

/** Pure startup policy shared by the UI and restart regression tests. */
enum class WalletStart { WELCOME, OPEN, UNLOCK }
fun walletStart(manifest: VaultManifest?): WalletStart = when {
    manifest == null -> WalletStart.WELCOME
    manifest.lockMode == AppLockMode.OFF -> WalletStart.OPEN
    else -> WalletStart.UNLOCK
}
