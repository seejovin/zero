package app.localwallet.mainnet

import android.database.sqlite.SQLiteDatabase
import android.system.Os
import android.system.OsConstants
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import uniffi.wallet_core.validateActivity

/** Additive v0.3 database. Existing encrypted vault and outgoing journal files are untouched. */
class ActivityStore(directory: File, private val address: String) : AutoCloseable {
    private val path = File(directory, "activity-v1.sqlite3")
    private val db = SQLiteDatabase.openDatabase(path.absolutePath, null, SQLiteDatabase.CREATE_IF_NECESSARY or SQLiteDatabase.NO_LOCALIZED_COLLATORS)
    init {
        db.enableWriteAheadLogging()
        db.execSQL("PRAGMA synchronous=FULL")
        db.execSQL("CREATE TABLE IF NOT EXISTS identity(address TEXT NOT NULL)")
        db.rawQuery("SELECT address FROM identity", null).use { cursor ->
            if (cursor.moveToFirst()) {
                if (!cursor.getString(0).equals(address, true)) throw WalletFailure("Activity database belongs to another wallet.")
            } else db.execSQL("INSERT INTO identity(address) VALUES(?)", arrayOf(address.lowercase()))
        }
        db.execSQL("CREATE TABLE IF NOT EXISTS observations(sequence INTEGER PRIMARY KEY AUTOINCREMENT, chain_id INTEGER NOT NULL, activity_id TEXT NOT NULL, kind TEXT NOT NULL, fingerprint TEXT NOT NULL, record_json TEXT NOT NULL, evidence_json TEXT NOT NULL, observed_at INTEGER NOT NULL, UNIQUE(chain_id,activity_id,fingerprint))")
        db.execSQL("CREATE INDEX IF NOT EXISTS observations_lookup ON observations(chain_id,activity_id,sequence)")
        db.execSQL("CREATE TRIGGER IF NOT EXISTS observations_no_update BEFORE UPDATE ON observations BEGIN SELECT RAISE(ABORT,'activity observations are immutable'); END")
        db.execSQL("CREATE TRIGGER IF NOT EXISTS observations_no_delete BEFORE DELETE ON observations BEGIN SELECT RAISE(ABORT,'activity observations are immutable'); END")
        db.execSQL("CREATE TABLE IF NOT EXISTS tracked(chain_id INTEGER NOT NULL, hash TEXT NOT NULL, added_at INTEGER NOT NULL, PRIMARY KEY(chain_id,hash))")
        db.execSQL("CREATE TABLE IF NOT EXISTS tracking_observations(sequence INTEGER PRIMARY KEY AUTOINCREMENT, chain_id INTEGER NOT NULL, hash TEXT NOT NULL, status TEXT NOT NULL, observed_at INTEGER NOT NULL)")
        db.execSQL("CREATE TABLE IF NOT EXISTS scans(sequence INTEGER PRIMARY KEY AUTOINCREMENT, chain_id INTEGER NOT NULL, from_block INTEGER NOT NULL, to_block INTEGER NOT NULL, note TEXT, observed_at INTEGER NOT NULL)")
        Os.chmod(path.absolutePath, 384)
        val fd = Os.open(directory.absolutePath, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
        try { Os.fsync(fd) } finally { Os.close(fd) }
    }
    private fun requireIdentity(chain: Long, activity: JSONObject) {
        if (chain !in NetworkPolicy.evmChainOrder || activity.getLong("chain_id") != chain || !activity.getString("address").equals(address, true)) {
            throw WalletFailure("Activity record network or account mismatch.")
        }
    }
    private fun sameRecord(actual:JSONObject, expected:JSONObject) {
        val keys=actual.keys().asSequence().toSet()
        if(keys != expected.keys().asSequence().toSet() || keys.any { actual.get(it).toString() != expected.get(it).toString() }) throw WalletFailure("Cached transfer fields do not match their saved evidence.")
    }
    private fun verifiedRecord(chain:Long, kind:String, text:String, evidenceText:String):JSONObject {
        val record=RpcJson.parse(text)
        requireIdentity(chain,record)
        val evidence=RpcJson.parse(evidenceText)
        if(kind == "observed") {
            val finalizedHeight=evidence.getLong("finalized_height")
            if(finalizedHeight < 0) throw WalletFailure("Invalid saved finality height.")
            val validated=JSONObject(validateActivity(chain.toULong(),address,evidence.getJSONObject("transaction").toString(),evidence.getJSONObject("receipt").toString(),evidence.getJSONObject("canonical_block").toString(),finalizedHeight.toULong()))
            val expected=validated.getJSONArray("activities").objects().firstOrNull { it.getString("id") == record.getString("id") }
                ?: throw WalletFailure("Cached transfer is absent from its saved receipt evidence.")
            sameRecord(record,expected)
        } else if(kind in setOf("reorg","conflict")) {
            val sequence=evidence.getLong("prior_observation_sequence")
            val prior=db.rawQuery("SELECT kind,record_json,evidence_json FROM observations WHERE sequence=? AND chain_id=? AND activity_id=?",arrayOf(sequence.toString(),chain.toString(),record.getString("id"))).use {
                if(!it.moveToFirst() || it.getString(0) != "observed") throw WalletFailure("Activity correction is missing its immutable prior observation.")
                verifiedRecord(chain,"observed",it.getString(1),it.getString(2))
            }
            sameRecord(record,prior)
            val canonical=evidence.getJSONObject("canonical_block")
            if(evidence.getString("prior_block_hash") != prior.getString("block_hash") || Amounts.longQuantity(canonical.getString("number")) != prior.getLong("block_number") || Amounts.hash(canonical.getString("hash")) == Amounts.hash(prior.getString("block_hash")) || (kind == "conflict") != (prior.getString("inclusion") == "finalized")) throw WalletFailure("Activity correction evidence is inconsistent.")
        } else throw WalletFailure("Unrecognized activity observation state.")
        return record
    }
    private fun digest(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it.toInt() and 255) }
    fun latest(chain: Long, limit: Int = 100): List<JSONObject> {
        val records = mutableListOf<JSONObject>()
        db.rawQuery("SELECT o.kind,o.record_json,o.observed_at,o.evidence_json FROM observations o INNER JOIN (SELECT activity_id,MAX(sequence) sequence FROM observations WHERE chain_id=? GROUP BY activity_id) newest ON o.sequence=newest.sequence ORDER BY o.sequence DESC LIMIT ?", arrayOf(chain.toString(), limit.coerceIn(1,500).toString())).use { cursor ->
            while (cursor.moveToNext()) {
                val record = verifiedRecord(chain,cursor.getString(0),cursor.getString(1),cursor.getString(3))
                record.put("verification_state", cursor.getString(0)).put("observed_at", cursor.getLong(2))
                records.add(record)
            }
        }
        return records.sortedByDescending { it.getLong("block_number") }
    }
    fun latestForHash(chain: Long, hash: String): List<JSONObject> {
        val records = mutableListOf<JSONObject>()
        db.rawQuery("SELECT o.kind,o.record_json,o.observed_at,o.evidence_json FROM observations o INNER JOIN (SELECT activity_id,MAX(sequence) sequence FROM observations WHERE chain_id=? AND activity_id LIKE ? GROUP BY activity_id) newest ON o.sequence=newest.sequence ORDER BY o.sequence",arrayOf(chain.toString(),"%:${Amounts.hash(hash)}:%")).use { cursor ->
            while(cursor.moveToNext()) {
                val record=verifiedRecord(chain,cursor.getString(0),cursor.getString(1),cursor.getString(3))
                records.add(record.put("verification_state",cursor.getString(0)).put("observed_at",cursor.getLong(2)))
            }
        }
        return records
    }
    fun knownHashes(chain: Long): Set<String> = latest(chain, 500).map { it.getString("transaction_hash").lowercase() }.toSet()
    fun observe(chain: Long, result: JSONObject, evidence: JSONObject) {
        val activities = result.getJSONArray("activities").objects()
        val evidenceBytes = evidence.toString().toByteArray(Charsets.UTF_8).size.toLong()
        if (evidenceBytes > 1_048_576 || evidenceBytes * activities.size > 4_194_304L) {
            throw WalletFailure("This transaction has too many transfer events to store here. Its on-chain balance remains independent of this activity list.")
        }
        db.beginTransaction()
        try {
            for ((hash, batch) in activities.groupBy { it.getString("transaction_hash") }) {
                val prior=latestForHash(chain,hash).filter { it.getString("verification_state") == "observed" && it.getString("block_hash").equals(batch.first().getString("block_hash"),true) }
                if(prior.isNotEmpty() && prior.map { it.getString("id") }.toSet() != batch.map { it.getString("id") }.toSet()) throw WalletFailure("Receipt changed the transfer set in an already verified block.")
            }
            for (activity in activities) {
                requireIdentity(chain, activity)
                val transactionPrior = latestForHash(chain,activity.getString("transaction_hash"))
                if (transactionPrior.any { it.getString("verification_state") == "conflict" }) throw WalletFailure("A finality contradiction needs investigation before this transaction can change.")
                if (transactionPrior.any { it.getString("verification_state") == "observed" && !it.getString("block_hash").equals(activity.getString("block_hash"),true) }) {
                    throw WalletFailure("A new activity ID cannot bypass prior canonical inclusion evidence.")
                }
                val old = last(chain, activity.getString("id"))
                if (old != null) {
                    if (old.first == "conflict") throw WalletFailure("A finality contradiction needs investigation before activity can be updated.")
                    if (old.first == "observed" && !old.second.getString("block_hash").equals(activity.getString("block_hash"), true)) {
                        throw WalletFailure("A changed inclusion needs a verified reorganization record first.")
                    }
                    if (old.first == "observed" && old.second.getString("inclusion") == "finalized" && activity.getString("inclusion") != "finalized") {
                        throw WalletFailure("A provider cannot silently downgrade finalized transfer evidence.")
                    }
                    if (old.first == "observed" && old.second.getString("block_hash").equals(activity.getString("block_hash"),true)) {
                        for (key in listOf("direction","asset","amount_units","from","to","transaction_hash","block_number","log_index","timestamp")) {
                            if (old.second.get(key).toString() != activity.get(key).toString()) throw WalletFailure("Provider disagrees with earlier verified transfer details.")
                        }
                    }
                }
                append(chain, activity, "observed", evidence)
            }
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
    }
    private fun last(chain: Long, id: String): Pair<String,JSONObject>? = db.rawQuery("SELECT kind,record_json FROM observations WHERE chain_id=? AND activity_id=? ORDER BY sequence DESC LIMIT 1", arrayOf(chain.toString(),id)).use {
        if (it.moveToFirst()) it.getString(0) to RpcJson.parse(it.getString(1)) else null
    }
    private fun append(chain: Long, activity: JSONObject, kind: String, evidence: JSONObject) {
        val clean = JSONObject(activity.toString()).apply { remove("verification_state"); remove("observed_at") }
        if (evidence.toString().length > 1_048_576) throw WalletFailure("Activity evidence exceeds the local record limit.")
        val fingerprint = digest(kind + "|" + clean.toString() + if(kind == "observed") "" else evidence.toString())
        db.execSQL("INSERT OR IGNORE INTO observations(chain_id,activity_id,kind,fingerprint,record_json,evidence_json,observed_at) VALUES(?,?,?,?,?,?,?)",
            arrayOf(chain, clean.getString("id"), kind, fingerprint, clean.toString(), evidence.toString(), System.currentTimeMillis()/1000))
    }
    fun markReorgGroup(chain:Long, prior:List<JSONObject>, canonical:JSONObject): Boolean {
        val conflict=prior.any { it.getString("inclusion") == "finalized" }
        db.beginTransaction()
        try {
            for(record in prior) {
                requireIdentity(chain,record)
                if(Amounts.longQuantity(canonical.getString("number")) != record.getLong("block_number") || canonical.getString("hash").equals(record.getString("block_hash"),true)) throw WalletFailure("Invalid reorganization evidence.")
                val previousSequence = db.rawQuery("SELECT sequence,kind FROM observations WHERE chain_id=? AND activity_id=? ORDER BY sequence DESC LIMIT 1",arrayOf(chain.toString(),record.getString("id"))).use {
                    if(!it.moveToFirst() || it.getString(1) != "observed") throw WalletFailure("Reorganization has no immutable prior observation.")
                    it.getLong(0)
                }
                append(chain,record,if(record.getString("inclusion") == "finalized") "conflict" else "reorg",JSONObject().put("prior_observation_sequence",previousSequence).put("prior_block_hash",record.getString("block_hash")).put("canonical_block",canonical))
            }
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
        return conflict
    }
    fun track(chain: Long, hash: String) {
        db.execSQL("INSERT OR IGNORE INTO tracked(chain_id,hash,added_at) VALUES(?,?,?)",arrayOf<Any>(chain,Amounts.hash(hash),System.currentTimeMillis()/1000))
    }
    fun trackingStatus(chain: Long, hash: String, status: String) {
        if (status !in setOf("pending","not_found","failed","not_recipient","verified","unavailable","conflict")) throw WalletFailure("Unknown tracking status.")
        val previous=db.rawQuery("SELECT status FROM tracking_observations WHERE chain_id=? AND hash=? ORDER BY sequence DESC LIMIT 1",arrayOf(chain.toString(),Amounts.hash(hash))).use { if(it.moveToFirst()) it.getString(0) else null }
        if(previous == status) return
        db.execSQL("INSERT INTO tracking_observations(chain_id,hash,status,observed_at) VALUES(?,?,?,?)",arrayOf<Any>(chain,Amounts.hash(hash),status,System.currentTimeMillis()/1000))
    }
    fun tracked(chain: Long): List<JSONObject> {
        val records=mutableListOf<JSONObject>()
        db.rawQuery("SELECT t.hash,(SELECT status FROM tracking_observations o WHERE o.chain_id=t.chain_id AND o.hash=t.hash ORDER BY sequence DESC LIMIT 1),(SELECT observed_at FROM tracking_observations o WHERE o.chain_id=t.chain_id AND o.hash=t.hash ORDER BY sequence DESC LIMIT 1) FROM tracked t WHERE chain_id=? ORDER BY added_at DESC LIMIT 30",arrayOf(chain.toString())).use {
            while(it.moveToNext()) records.add(JSONObject().put("hash",it.getString(0)).put("status", if(it.isNull(1)) "pending" else it.getString(1)).put("observed_at",if(it.isNull(2)) 0 else it.getLong(2)))
        }
        return records
    }
    fun coverage(chain: Long): Triple<Long,Long,String?>? = db.rawQuery("SELECT from_block,to_block,note FROM scans WHERE chain_id=? ORDER BY sequence DESC LIMIT 1",arrayOf(chain.toString())).use {
        if(it.moveToFirst()) Triple(it.getLong(0),it.getLong(1),if(it.isNull(2)) null else it.getString(2)) else null
    }
    fun scanned(chain: Long, from: Long, to: Long, note: String?) {
        db.execSQL("INSERT INTO scans(chain_id,from_block,to_block,note,observed_at) VALUES(?,?,?,?,?)",arrayOf<Any?>(chain,from,to,note,System.currentTimeMillis()/1000))
    }
    fun snapshot(chain: Long, note: String?=null): ActivitySnapshot {
        val coverage=coverage(chain)
        return ActivitySnapshot(latest(chain),tracked(chain),coverage?.first,coverage?.second,note ?: coverage?.third)
    }
    fun audit(chain:Long): JSONArray {
        val result=JSONArray()
        db.rawQuery("SELECT sequence,kind,record_json,evidence_json,observed_at FROM observations WHERE chain_id=? ORDER BY sequence",arrayOf(chain.toString())).use {
            while(it.moveToNext()) result.put(JSONObject().put("sequence",it.getLong(0)).put("kind",it.getString(1)).put("record",RpcJson.parse(it.getString(2))).put("evidence",RpcJson.parse(it.getString(3))).put("observed_at",it.getLong(4)))
        }
        return result
    }
    override fun close()=db.close()
}
