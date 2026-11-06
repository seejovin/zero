#!/usr/bin/env python3
"""Exercise the generated Solana/TRON UniFFI bridge and durable journal offline.

Only the deliberately public, unfunded BIP39 fixture is used. No endpoint is
contacted, and synthetic Solana blockhashes/TRON TAPOS cannot be submitted.

Generate vectors with `cargo run --locked --quiet --example
solana_differential_vectors > /tmp/solana-vectors.json`, then run:

python scripts/check-family-bridge.py --bindings /path/to/generated/python \
    --solana-vectors /tmp/solana-vectors.json
"""

import argparse
import base64
import gc
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import time


def encode(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True)


def main():
    if not __debug__:
        raise RuntimeError("Run this verification without Python optimization; assertions are required")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bindings", type=Path, required=True)
    parser.add_argument("--solana-vectors", type=Path, required=True)
    args = parser.parse_args()
    binding_path = args.bindings.resolve()
    sys.path.insert(0, str(binding_path))
    import wallet_core as wallet  # Generated code and its exact host library.

    source = json.loads(args.solana_vectors.read_text())
    assert source["fixture"] == "PUBLIC UNFUNDED OFFLINE ONLY"
    assert len(source["vectors"]) == 5
    assert source["derivation_path"] == "m/44'/501'/0'/0'"
    phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    assert source["phrase"] == phrase
    solana_owner = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"
    tron_owner = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH"
    tron_recipient = "TDp4x8kCJ6aHz8wnqZgkaVQAm2ajH1og3L"
    key = bytes([0x42] * 32)
    restored = json.loads(wallet.restore_wallet(phrase, key))
    vault_hex = restored["vault_hex"]
    assert "mnemonic" not in restored
    assert wallet.solana_address(vault_hex, key) == solana_owner
    assert wallet.tron_address(vault_hex, key) == tron_owner
    assert wallet.solana_validate_address(solana_owner) == solana_owner
    assert wallet.tron_normalize_address(tron_owner) == tron_owner
    assert wallet.tron_address_hex(tron_recipient) == "412a260a110bc7b03f19c40a0bd04ff2c5dcb57594"

    rejected = 0

    def reject(operation):
        nonlocal rejected
        try:
            operation()
        except (wallet.WalletError.InvalidInput, wallet.WalletError.Policy,
                wallet.WalletError.Conflict, wallet.WalletError.Crypto,
                wallet.WalletError.Storage):
            rejected += 1
            return
        raise AssertionError("Generated family FFI accepted a forbidden operation")

    reject(lambda: wallet.solana_address(vault_hex, bytes([0x43] * 32)))
    reject(lambda: wallet.tron_address(vault_hex, bytes([0x43] * 32)))
    reject(lambda: wallet.solana_validate_address(tron_owner))
    reject(lambda: wallet.tron_normalize_address(solana_owner))

    cases = []
    for index, original in enumerate(source["vectors"]):
        old_prepared = original["prepared"]
        old_signed = original["signed"]
        # Historic signatures/approvals must remain readable after their lifetime.
        assert json.loads(wallet.solana_validate_prepared(encode(old_prepared))) == old_prepared
        assert json.loads(wallet.solana_validate_signed(encode(old_prepared), encode(old_signed))) == old_signed
        # A fresh explicit review is used only for this offline signing test. Its
        # message is unchanged, so independent JS-verified signatures must match.
        prepared_json = wallet.solana_prepare(encode(old_prepared["request"]))
        prepared = json.loads(prepared_json)
        assert prepared["unsigned_transaction"] == old_prepared["unsigned_transaction"]
        assert prepared["amount_units"] == old_prepared["amount_units"]
        assert json.loads(wallet.solana_validate_prepared(prepared_json)) == prepared
        signed_json = wallet.solana_sign(vault_hex, key, prepared_json, prepared["approval_digest"])
        signed = json.loads(signed_json)
        assert signed["raw_transaction"] == old_signed["raw_transaction"]
        assert signed["hash"] == old_signed["hash"]
        assert json.loads(wallet.solana_validate_signed(prepared_json, signed_json)) == signed
        if prepared["request"]["asset"] != "SOL":
            assert wallet.solana_token_account(prepared["request"]["to"], prepared["request"]["asset"]) == prepared["destination_token_account"]
        changed = dict(signed)
        raw = bytearray(base64.b64decode(changed["raw_transaction"], validate=True))
        raw[1] ^= 1
        changed["raw_transaction"] = base64.b64encode(raw).decode("ascii")
        reject(lambda: wallet.solana_validate_signed(prepared_json, encode(changed)))
        cases.append((f"solana-{index}-{prepared['request']['asset']}", prepared_json, signed_json))

    for asset in ("TRX", "USDT"):
        observed_ms = time.time_ns() // 1_000_000
        request = {
            "intent_id": f"public-family-bridge-tron-{asset.lower()}",
            "network": "tron-mainnet", "from": tron_owner, "to": tron_recipient,
            "asset": asset, "amount": "0.010001", "maximum": False,
            "balance_sun": "1000000000", "token_balance_units": "10001",
            "ref_block_id": f"{80_000_000:016x}" + "12" * 24,
            "ref_block_number": 80_000_000,
            "ref_block_timestamp_ms": observed_ms - 1000,
            "observed_at_ms": observed_ms, "recipient_active": True,
            "staked_bandwidth": 0, "free_bandwidth": 0, "available_energy": 0,
            "bandwidth_price_sun": "1000", "energy_price_sun": "100",
            "creation_fee_sun": "1000000", "creation_bandwidth_fee_sun": "100000",
            "creation_bandwidth_rate": 1, "max_fee_limit_sun": "15000000000",
            "max_energy_factor": 34000,
            "energy_used": 65000 if asset == "USDT" else 0,
            "energy_penalty": 35000 if asset == "USDT" else 0,
        }
        prepared_json = wallet.tron_prepare(encode(request))
        prepared = json.loads(prepared_json)
        assert prepared["request"] == request
        assert prepared["amount_units"] == "10001"
        assert prepared["hash"] == hashlib.sha256(bytes.fromhex(prepared["raw_data_hex"])).hexdigest()
        assert json.loads(wallet.tron_validate_prepared(prepared_json)) == prepared
        signed_json = wallet.tron_sign(vault_hex, key, prepared_json, prepared["approval_digest"])
        signed = json.loads(signed_json)
        assert signed["hash"] == prepared["hash"]
        assert signed["amount_units"] == "10001"
        assert json.loads(wallet.tron_validate_signed(signed_json)) == signed
        value = signed["broadcast"]["raw_data"]["contract"][0]["parameter"]["value"]
        if asset == "TRX":
            assert value["amount"] == 10001
            assert "fee_limit" not in signed["broadcast"]["raw_data"]
        else:
            assert value["contract_address"] == wallet.tron_address_hex("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")
            assert value["data"] == "a9059cbb" + wallet.tron_address_hex(tron_recipient)[2:].zfill(64) + f"{10001:064x}"
            assert prepared["fee_limit_sun"] == "15840000"
        changed = dict(signed)
        raw = bytearray.fromhex(changed["raw_transaction"])
        raw[-1] ^= 1
        changed["raw_transaction"] = raw.hex()
        reject(lambda: wallet.tron_validate_signed(encode(changed)))
        cases.append((f"tron-{asset}", prepared_json, signed_json))

    reopen_count = 0
    with tempfile.TemporaryDirectory(prefix="local-wallet-family-bridge-") as directory:
        for name, prepared_json, signed_json in cases:
            prepared, signed = json.loads(prepared_json), json.loads(signed_json)
            request = prepared["request"]
            identity, network, owner = request["intent_id"], request["network"], request["from"]
            path = str(Path(directory) / f"{name}.sqlite3")
            journal = wallet.FamilyJournal(path, network, owner)
            assert json.loads(journal.record_prepared(prepared_json))["phase"] == "prepared"
            assert json.loads(journal.record_signed(identity, signed_json))["phase"] == "signed"
            before = json.loads(journal.get_payment(identity))
            assert before["record"]["hash"] == signed["hash"]
            assert json.loads(before["signed_json"])["raw_transaction"] == signed["raw_transaction"]
            assert json.loads(before["prepared_json"]) == prepared
            # Public audit views must not expose reusable signed bytes or vault data.
            for audit in (journal.list_payments(), journal.list_events(identity)):
                assert signed["raw_transaction"] not in audit
                assert vault_hex not in audit and phrase not in audit
                assert "signed_json" not in audit
            del journal
            gc.collect()

            journal = wallet.FamilyJournal(path, network, owner)
            reopen_count += 1
            assert json.loads(journal.get_payment(identity)) == before
            assert json.loads(journal.mark_submission(identity, "unknown"))["phase"] == "unknown"
            unknown = json.loads(journal.get_payment(identity))
            assert unknown["signed_json"] == before["signed_json"]
            assert unknown["prepared_json"] == before["prepared_json"]
            reject(lambda: journal.cancel_prepared(identity))
            events = journal.list_events(identity)
            journal.mark_submission(identity, "unknown")
            assert journal.list_events(identity) == events
            del journal
            gc.collect()

            journal = wallet.FamilyJournal(path, network, owner)
            reopen_count += 1
            after = json.loads(journal.get_payment(identity))
            assert after == unknown
            assert after["record"]["phase"] == "unknown"
            assert after["record"]["amount_units"] == prepared["amount_units"]
            assert after["record"]["hash"] == signed["hash"]
            assert after["signed_json"] == before["signed_json"]
            assert json.loads(after["signed_json"])["raw_transaction"] == signed["raw_transaction"]
            assert journal.list_events(identity) == events
            if network == "solana-mainnet":
                wallet.solana_validate_signed(after["prepared_json"], after["signed_json"])
            else:
                wallet.tron_validate_signed(after["signed_json"])
            del journal
            gc.collect()

            wrong_network = "tron-mainnet" if network == "solana-mainnet" else "solana-mainnet"
            wrong_owner = tron_owner if network == "solana-mainnet" else solana_owner
            reject(lambda: wallet.FamilyJournal(path, wrong_network, wrong_owner))

    assert len(cases) == 7 and reopen_count == 14
    assert rejected == 25
    library = binding_path / "libwallet_core.so"
    print("PASS: actual generated Python UniFFI bridge authenticated the public BIP39 fixture and derived both expected family addresses.")
    print("PASS: 5 Solana vectors retained independently verified message/signature bytes; 2 fresh synthetic TRON fixtures verified native TRX and canonical USDT wire data and exact units.")
    print("PASS: 7 family journals persisted signed and unknown states; 14 reopens preserved every prepared JSON, signed JSON, raw transaction byte, transaction identifier and event record.")
    print(f"PASS: {rejected} wrong-key, cross-family, signature-mutation, signed-cancellation and wrong-journal-identity operations were rejected; duplicate unknown observations were idempotent.")
    print("No RPC, Android runtime, real TAPOS/blockhash, live signature request or network submission was used. All signed material is an unfunded public offline fixture.")
    if library.exists():
        print("Host library SHA-256:", hashlib.sha256(library.read_bytes()).hexdigest())


if __name__ == "__main__":
    main()
