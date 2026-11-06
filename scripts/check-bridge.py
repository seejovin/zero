#!/usr/bin/env python3
"""Exercise actual generated UniFFI bindings with public, unfunded fixtures.

This host check does not contact RPC providers or exercise Android/JVM code.
"""
import argparse
import gc
import json
from pathlib import Path
import sys
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--bindings", type=Path, required=True)
args = parser.parse_args()
sys.path.insert(0, str(args.bindings.resolve()))
import wallet_core as wallet  # noqa: E402 - generated bindings are a test input

key = bytes(range(32))
# Standard public development fixture. It has no secrecy and is unsafe for funds.
fixture = "test test test test test test test test test test test junk"
restored = json.loads(wallet.restore_wallet(fixture, key))
assert restored["address"] == "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
assert "mnemonic" not in restored
assert wallet.wallet_address(restored["vault_hex"], key) == restored["address"]

recipient = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
block_hash = "0x" + "11" * 32
transfer_topic = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
# Independent public registry fixtures; do not derive expected contracts from Rust.
contracts = {
    1: {
        "ETH": None,
        "USDC": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        "EURC": "0x1aBaEA1f7C830bD89Acc67eC4af516284b1bC33c",
        "USDT": "0xdAC17F958D2ee523a2206206994597C13D831ec7",
    },
    8453: {
        "ETH": None,
        "USDC": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        "EURC": "0x60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42",
    },
    10: {
        "ETH": None,
        "USDC": "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85",
    },
    42161: {
        "ETH": None,
        "USDC": "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
    },
    43114: {
        "AVAX": None,
        "USDC": "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E",
        "EURC": "0xC891EB4cbdEFf6e073e859e987815Ed1505c2ACD",
        "USDT": "0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7",
    },
    137: {
        "POL": None,
        "USDC": "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359",
    },
    56: {
        "BNB": None,
    },
}
native_assets = {
    1: "ETH", 8453: "ETH", 10: "ETH", 42161: "ETH",
    43114: "AVAX", 137: "POL", 56: "BNB",
}

# Check the generated registry against independent, fixed asset identities.
# BNB Smart Chain intentionally has no Binance-Peg stablecoins in this registry.
registry = json.loads(wallet.asset_registry())["networks"]
assert len(registry) == len(contracts) == 7
assert {network["chain_id"] for network in registry} == set(contracts)
for network in registry:
    chain = network["chain_id"]
    assets = network["assets"]
    assert len(assets) == len(contracts[chain])
    assert {asset["symbol"] for asset in assets} == set(contracts[chain])
    for asset in assets:
        expected = contracts[chain][asset["symbol"]]
        actual = asset["contract"]
        assert (actual.lower() if actual else None) == (expected.lower() if expected else None)
        assert asset["decimals"] == (18 if asset["symbol"] == native_assets[chain] else 6)


def expect_rejection(operation):
    try:
        operation()
    except (
        wallet.WalletError.Policy,
        wallet.WalletError.InvalidInput,
        wallet.WalletError.Conflict,
    ):
        return
    raise AssertionError("The generated FFI accepted a forbidden payment or receipt")


def payment_request(chain, asset, nonce=0):
    rollup = chain in (8453, 10)
    arbitrum = chain == 42161
    native = asset == native_assets.get(chain)
    bnb = chain == 56
    return {
        "intent_id": f"bridge-{chain}-{asset}",
        "chain_id": chain,
        "from": restored["address"],
        "to": recipient,
        "asset": asset,
        "amount": "0.010000000000000001" if native else "0.010001",
        "nonce": nonce,
        "gas_limit": 250000 if arbitrum else (21000 if native else 80000),
        "max_fee_per_gas": "20000000" if arbitrum else "2000000000",
        # BSC type-2 transactions use baseFee=0 and equal reviewed gas-price caps.
        "max_priority_fee_per_gas": "0" if arbitrum else ("2000000000" if bnb else "1000000000"),
        "l1_fee_estimate_wei": "1200000" if rollup else "0",
        "operator_fee_estimate_wei": "42000" if rollup else "0",
    }


def receipt_for(chain, signed, transaction_to):
    native = signed["asset"] == native_assets[chain]
    receipt = {
        "transactionHash": signed["hash"],
        "blockHash": block_hash,
        "blockNumber": "0x64",
        "from": signed["from"],
        "to": transaction_to,
        "status": "0x1",
        "gasUsed": hex(200000 if chain == 42161 else (21000 if native else 55000)),
        "effectiveGasPrice": hex(10000000 if chain == 42161 else (2000000000 if chain == 56 else 1000000000)),
        "logs": [],
        "type": "0x2",
        "contractAddress": None,
    }
    if chain in (8453, 10):
        receipt.update(l1Fee=hex(1000000), operatorFee=hex(42000))
    if chain == 42161:
        # An informational subset of gasUsed, never an additional fee.
        receipt["gasUsedForL1"] = "0x10000"
    if not native:
        receipt["logs"] = [{
            "address": transaction_to,
            "topics": [
                transfer_topic,
                "0x" + signed["from"][2:].lower().zfill(64),
                "0x" + recipient[2:].lower().zfill(64),
            ],
            "data": f"0x{int(signed['amount_units']):064x}",
            "transactionHash": signed["hash"],
            "blockHash": block_hash,
            "blockNumber": "0x64",
            "logIndex": "0x0",
            "removed": False,
        }]
    return receipt


cases = 0
with tempfile.TemporaryDirectory(prefix="local-wallet-bridge-") as directory:
    for chain, assets in contracts.items():
        path = str(Path(directory) / f"journal-{chain}.sqlite3")
        journal = wallet.WalletJournal(path)
        saved = []
        for nonce, (asset, contract) in enumerate(assets.items()):
            request = payment_request(chain, asset, nonce)
            prepared_json = wallet.prepare_payment(json.dumps(request))
            prepared = json.loads(prepared_json)
            assert json.loads(wallet.validate_prepared_json(prepared_json))["request"] == request
            native = asset == native_assets[chain]
            assert native == (contract is None)
            units = "10000000000000001" if native else "10001"
            assert prepared["amount_units"] == units
            execution_cap = request["gas_limit"] * int(request["max_fee_per_gas"])
            extra = int(request["l1_fee_estimate_wei"]) + int(request["operator_fee_estimate_wei"])
            assert prepared["max_fee_wei"] == str(execution_cap)
            assert prepared["estimated_total_fee_wei"] == str(execution_cap + extra)
            assert prepared["fee_reserve_wei"] == str(execution_cap + 2 * extra)
            signed_json = wallet.sign_payment(
                restored["vault_hex"], key, prepared_json, prepared["approval_digest"]
            )
            signed = json.loads(signed_json)
            wallet.validate_signed_for_prepared(prepared_json, signed_json)
            inspected = json.loads(wallet.inspect_signed(signed["raw_transaction"]))
            assert inspected["hash"] == signed["hash"]
            assert inspected["chain_id"] == chain
            assert inspected["asset"] == asset
            assert inspected["amount_units"] == units
            # The inspector returns EIP-55 case; issuer registries may use a
            # different presentation of the same 20 address bytes.
            assert inspected["transaction_to"].lower() == (contract or recipient).lower(), (chain, asset)
            if chain == 56:
                assert request["max_fee_per_gas"] == request["max_priority_fee_per_gas"]
            receipt = receipt_for(chain, signed, inspected["transaction_to"])
            checked = json.loads(wallet.validate_receipt(signed_json, json.dumps(receipt), block_hash, 100))
            actual_execution = (2000000000000 if chain == 42161 else
                                42000000000000 if chain == 56 else
                                21000000000000 if native else 55000000000000)
            actual_extra = 1042000 if chain in (8453, 10) else 0
            assert checked["phase"] == "finalized" and checked["execution"] == "success"
            assert checked["actual_execution_fee_wei"] == str(actual_execution)
            assert checked["actual_fee_wei"] == str(actual_execution + actual_extra)
            assert checked["actual_l1_fee_wei"] == ("1000000" if chain in (8453, 10) else "0")
            assert checked["actual_operator_fee_wei"] == ("42000" if chain in (8453, 10) else "0")
            if chain in (8453, 10):
                for field in ("l1Fee", "operatorFee"):
                    missing = dict(receipt)
                    del missing[field]
                    expect_rejection(lambda: wallet.validate_receipt(signed_json, json.dumps(missing), block_hash, 100))
            elif chain == 42161:
                for field in ("l1Fee", "operatorFee"):
                    duplicate = dict(receipt, **{field: "0x1"})
                    expect_rejection(lambda: wallet.validate_receipt(signed_json, json.dumps(duplicate), block_hash, 100))
                for field in ("l1_fee_estimate_wei", "operator_fee_estimate_wei"):
                    duplicate = dict(request, **{field: "1"})
                    expect_rejection(lambda: wallet.prepare_payment(json.dumps(duplicate)))
            journal.record_prepared(prepared_json)
            record = json.loads(journal.record_signed(signed_json))
            assert record["raw_transaction"] == signed["raw_transaction"]
            assert record["phase"] == "signed"
            assert json.loads(journal.mark_submission(request["intent_id"], "unknown"))["phase"] == "unknown"
            saved.append((request["intent_id"], signed, receipt, checked["actual_fee_wei"]))
            cases += 1
        del journal
        gc.collect()

        reopened = wallet.WalletJournal(path)
        for intent, signed, receipt, expected_fee in saved:
            record = json.loads(reopened.get_payment(intent))
            assert record["hash"] == signed["hash"]
            assert record["raw_transaction"] == signed["raw_transaction"]
            assert record["phase"] == "unknown"
            included = json.loads(reopened.observe_receipt(intent, json.dumps(receipt), block_hash, 99))
            assert included["phase"] == "included" and included["execution"] == "success"
            final = json.loads(reopened.observe_receipt(intent, json.dumps(receipt), block_hash, 100))
            assert final["phase"] == "finalized" and final["actual_fee_wei"] == expected_fee
            events = json.loads(reopened.list_events(intent))
            assert len(events) == 5
            reopened.observe_receipt(intent, json.dumps(receipt), block_hash, 100)
            assert json.loads(reopened.list_events(intent)) == events
        del reopened
        gc.collect()

        final_reopen = wallet.WalletJournal(path)
        for intent, signed, _, expected_fee in saved:
            record = json.loads(final_reopen.get_payment(intent))
            assert record["phase"] == "finalized"
            assert record["raw_transaction"] == signed["raw_transaction"]
            assert record["actual_fee_wei"] == expected_fee
        del final_reopen
        gc.collect()

for chain in (10, 42161, 137, 56):
    expect_rejection(lambda: wallet.prepare_payment(json.dumps(payment_request(chain, "EURC"))))
for asset in ("USDC", "USDT"):
    expect_rejection(lambda: wallet.prepare_payment(json.dumps(payment_request(56, asset))))
for chain in (43114, 137, 56):
    expect_rejection(lambda: wallet.prepare_payment(json.dumps(payment_request(chain, "ETH"))))
for chain, native in ((11155111, "ETH"), (84532, "ETH"), (11155420, "ETH"),
                      (421614, "ETH"), (43113, "AVAX"), (80002, "POL"), (97, "BNB")):
    expect_rejection(lambda: wallet.prepare_payment(json.dumps(payment_request(chain, native))))
bnb_request = payment_request(56, "BNB")
unequal_caps = dict(bnb_request, max_priority_fee_per_gas="1000000000")
expect_rejection(lambda: wallet.prepare_payment(json.dumps(unequal_caps)))
excessive_price = dict(bnb_request, max_fee_per_gas="10000000001", max_priority_fee_per_gas="10000000001")
expect_rejection(lambda: wallet.prepare_payment(json.dumps(excessive_price)))

assert cases == 18
print(
    "PASS: actual generated UniFFI host bridge authenticated the public fixture; "
    "verified 7 EVM networks and 18 canonical asset positions; signed and inspected "
    "18 ETH/AVAX/POL/BNB/USDC/EURC/USDT fixtures across Ethereum, Base, OP Mainnet, "
    "Arbitrum One, Avalanche C-Chain, Polygon PoS and BNB Smart Chain; preserved "
    "every native/token atomic unit; checked OP Stack receipt fees, Arbitrum "
    "single-charge fees and BNB equal gas-price caps; reopened each network "
    "journal through unknown and final states without changing signed bytes; "
    "rejected unsupported assets, wrong native assets, Binance-Peg stablecoin "
    "substitution, invalid BNB fee caps and all seven testnets. "
    "No RPC, Android runtime or live payment was used."
)
