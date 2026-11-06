#!/usr/bin/env python3
"""Actual generated UniFFI private-journal/OHTTP checks; no network or user wallet.

All envelopes below deliberately contain invalid synthetic proof/calldata bytes.
These checks validate FFI policy, durability and encryption, NOT ZK validity.
"""
import argparse
import gc
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import time


def encoded(value):
    return json.dumps(value, separators=(",", ":"))


def h(value):
    return "0x" + f"{value:064x}"


def main():
    if not __debug__:
        raise RuntimeError("Assertions must be enabled for verification")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bindings", type=Path, required=True)
    args = parser.parse_args()
    sys.path.insert(0, str(args.bindings.resolve()))
    import wallet_core as wallet

    rejected = 0

    def reject(function):
        nonlocal rejected
        try:
            function()
        except (wallet.WalletError.InvalidInput, wallet.WalletError.Policy,
                wallet.WalletError.Conflict, wallet.WalletError.Crypto, wallet.WalletError.Storage):
            rejected += 1
            return
        raise AssertionError("Private bridge accepted a forbidden operation")

    cap = json.loads(wallet.private_capabilities())
    assert cap["enabled"] is False and cap["proof_verification"] is False
    assert cap["public_fallback"] is False and cap["ohttp_client_library"] is True
    maximum = {"input_notes": [{"commitment": h(1), "nullifier": h(2),
               "units": "9007199254740993123456"}], "fee_units": "12345", "max_inputs": 1}
    result = json.loads(wallet.private_maximum_transfer(encoded(maximum)))
    assert result["recipient_units"] == "9007199254740993111111"
    assert result["change_units"] == "0"
    reject(lambda: wallet.private_maximum_transfer(encoded({**maximum, "recipient": "forbidden"})))
    reject(lambda: wallet.private_maximum_transfer(encoded({**maximum, "max_inputs": 0})))

    now = int(time.time())
    domain = {"protocol": "railgun-v2-candidate", "chain_id": 42161,
              "pool_contract": "0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9",
              "token_contract": "0xaf88d065e77c8cc2239327c5edb3a432268e5831", "token_decimals": 6,
              "verifier_digest": h(200)}
    fee = {"schedule_id": h(201), "fee_units": "20000", "fee_recipient_commitment": h(202),
           "fee_output_commitment": h(80), "max_native_fee_wei": "1000000000000000",
           "gas_limit": 2000000, "quote_valid_until": now + 600}
    draft = {"schema_version": 2, "intent_id": h(9), "tree_number": 0, "domain": domain,
             "recipient": "0zk-private-fixture-no-real-recipient", "amount_units": "100103456",
             "fee": fee, "input_notes": [{"commitment": h(20), "nullifier": h(40), "units": "100123456"}],
             "state_root": h(203), "output_commitments": [h(60), h(80)],
             "recipient_output_commitment": h(60), "change_units": "0", "change_output_commitment": None,
             "padding_output_commitments": [], "created_at": now, "approval_digest": ""}
    intent_json = wallet.private_prepare_intent(encoded(draft))
    intent = json.loads(intent_json)
    assert intent["approval_digest"].startswith("0x")
    reject(lambda: wallet.private_prepare_intent(encoded({**draft, "intent_id": "a-uuid-is-not-a-native-identity"})))
    reject(lambda: wallet.private_prepare_intent(encoded({**draft, "amount_units": "100103455"})))
    e = {"schema_version": 2, "domain": domain, "tree_number": 0, "state_root": h(203), "nullifiers": [h(40)],
         "output_commitments": [h(60), h(80)], "encrypted_outputs": ["0x" + "ab" * 64] * 2,
         "proof": "0x" + "cd" * 256, "call_data": "0x" + "ef" * 128, "fee": fee, "operation_digest": ""}
    envelope_json = wallet.private_seal_envelope(encoded(e))
    envelope = json.loads(envelope_json)
    assert wallet.private_validate_envelope(envelope_json) == envelope_json
    reject(lambda: wallet.private_seal_envelope(encoded({**e, "recipient": draft["recipient"]})))
    reject(lambda: wallet.private_validate_envelope(encoded({**envelope, "call_data": "0x01020304"})))
    reject(lambda: wallet.private_validate_envelope(envelope_json.replace('"schema_version":2', '"schema_version":2,"schema_version":2', 1)))

    with tempfile.TemporaryDirectory(prefix="private-ffi-offline-") as temporary:
        path = str(Path(temporary) / "private.sqlite3")
        key = bytes([0x42] * 32)
        journal = wallet.PrivatePaymentJournal(path, h(240), key)
        assert json.loads(journal.record_approved(intent_json))["phase"] == "approved"
        assert json.loads(journal.record_ready(h(9), envelope_json))["phase"] == "ready"
        assert json.loads(journal.mark_released(h(9)))["release_count"] == 1
        assert json.loads(journal.mark_unknown(h(9)))["phase"] == "unknown"
        saved = json.loads(journal.get_payment(h(9)))
        assert saved["intent_json"] == intent_json and saved["envelope_json"] == envelope_json
        reject(lambda: journal.cancel_approved(h(9)))
        duplicate = {**draft, "intent_id": h(10)}
        reject(lambda: journal.record_approved(wallet.private_prepare_intent(encoded(duplicate))))
        del journal
        gc.collect()
        for file in Path(temporary).iterdir():
            data = file.read_bytes()
            for private_string in (draft["recipient"], draft["amount_units"], envelope["proof"]):
                assert private_string.encode() not in data
        reject(lambda: wallet.PrivatePaymentJournal(path, h(240), bytes([0x43] * 32)))
        reopened = wallet.PrivatePaymentJournal(path, h(240), key)
        recovered = json.loads(reopened.get_payment(h(9)))
        assert recovered["record"]["phase"] == "unknown"
        assert recovered["envelope_json"] == envelope_json
        assert json.loads(reopened.mark_released(h(9)))["release_count"] == 2
        assert len(json.loads(reopened.list_events(h(9)))) == 5
        del reopened
        gc.collect()

    # RFC 7748's public test-vector key, never a user key or deployed service key.
    public = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
    config = bytes.fromhex("010020" + public + "000400010001")
    pin = hashlib.sha256(config).digest()
    gateway = wallet.PrivateOhttpGateway("gas.example.org", config, pin)
    reject(lambda: wallet.PrivateOhttpGateway("gas.example.org", config, bytes(32)))
    reject(lambda: wallet.PrivateOhttpGateway("user@gas.example.org", config, pin))
    first = gateway.seal_submission(envelope_json)
    second = gateway.seal_submission(envelope_json)
    first_bytes = bytes(first.ciphertext())
    assert first_bytes != bytes(second.ciphertext())
    assert envelope_json.encode() not in first_bytes
    reject(lambda: first.open_response(200, "application/json", b"{}"))
    reject(lambda: first.ciphertext())
    reject(lambda: first.open_response(200, "message/ohttp-res", b"{}"))
    reject(lambda: gateway.seal_submission(intent_json))
    reject(lambda: gateway.seal_status("not-an-operation-digest"))
    assert len(gateway.seal_fee_schedule().ciphertext()) > 4096
    print(encoded({"kind": "actual generated UniFFI private bridge", "passed": True,
                   "rejection_checks": rejected, "encrypted_journal_reopened": True,
                   "ohttp_ciphertext_generated": True, "single_use_context_checked": True,
                   "valid_zk_proof_tested": False, "network_requests": 0, "user_wallets_accessed": 0}))


if __name__ == "__main__":
    main()
