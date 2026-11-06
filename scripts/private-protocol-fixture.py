#!/usr/bin/env python3
"""Closed bridge driver for public, unfunded protocol qualification fixtures."""
import gc
import json
from pathlib import Path
import sys
import tempfile

if len(sys.argv) != 2:
    raise ValueError("Expected generated-binding directory")
sys.path.insert(0, str(Path(sys.argv[1]).resolve()))
import wallet_core as wallet

request = json.loads(sys.stdin.buffer.read(1_048_577))
identity = "0x" + "33" * 32
operation = request["operation"]
encode = lambda value: json.dumps(value, separators=(",", ":"))
if operation == "derive":
    # BIP39's published test mnemonic. This wallet must never hold funds.
    fixture = " ".join(["abandon"] * 11 + ["about"])
    vault_key = bytes([0x41] * 32)
    record = json.loads(wallet.restore_wallet(fixture, vault_key))
    keys = bytes(wallet.private_engine_keys(record["vault_hex"], vault_key, identity))
    assert len(keys) == 96
    result = {"wallet_identity": identity, "spending_private_key_hex": keys[:32].hex(),
              "viewing_private_key_hex": keys[32:64].hex(), "encrypted_state_key_hex": keys[64:].hex()}
elif operation == "seal_intent":
    result = {"intent_json": wallet.private_prepare_intent(request["intent_json"])}
elif operation == "seal_envelope":
    result = {"envelope_json": wallet.private_seal_envelope(request["envelope_json"])}
elif operation == "journal":
    with tempfile.TemporaryDirectory(prefix="private-protocol-fixture-") as directory:
        path = str(Path(directory) / "private.sqlite3")
        key = bytes([0x51] * 32)
        intent = json.loads(request["intent_json"])
        journal = wallet.PrivatePaymentJournal(path, identity, key)
        journal.record_approved(request["intent_json"])
        journal.record_ready(intent["intent_id"], request["envelope_json"])
        journal.mark_released(intent["intent_id"])
        journal.mark_unknown(intent["intent_id"])
        del journal
        gc.collect()
        reopened = wallet.PrivatePaymentJournal(path, identity, key)
        saved = json.loads(reopened.get_payment(intent["intent_id"]))
        assert saved["envelope_json"] == request["envelope_json"]
        assert saved["intent_json"] == request["intent_json"]
        result = {"phase": saved["record"]["phase"], "exact_envelope_preserved": True}
        del reopened
        gc.collect()
elif operation == "verify_schedule":
    result = {"schedule_json": wallet.private_verify_fee_schedule(
        request["packet_json"], bytes.fromhex(request["public_key"]), request["now"])}
else:
    raise ValueError("Unsupported fixture operation")
sys.stdout.write(encode(result))
