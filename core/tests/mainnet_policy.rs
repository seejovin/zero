//! All signatures use a public Hardhat development key. No RPC or broadcast.
use primitive_types::U256;
use serde_json::{json, Value};
use std::sync::OnceLock;
use wallet_core::{crypto, journal::Journal};

const FROM: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const TO: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

fn json_value(input: String) -> Value {
    serde_json::from_str(&input).unwrap()
}
fn request(chain: u64, asset: &str) -> Value {
    json!({"intent_id":"public-mainnet-fixture","chain_id":chain,"from":FROM,"to":TO,
        "asset":asset,"amount":"1","nonce":2,"gas_limit":if matches!(asset, "ETH" | "AVAX" | "POL" | "BNB") {21000} else {80000},
        "max_fee_per_gas":"2000000000","max_priority_fee_per_gas":if chain == 56 {"2000000000"} else {"1000000000"},
        "l1_fee_estimate_wei":if matches!(chain, 8453 | 10) {"1000"} else {"0"},
        "operator_fee_estimate_wei":if matches!(chain, 8453 | 10) {"100"} else {"0"}})
}
fn prepare(chain: u64, asset: &str) -> String {
    wallet_core::prepare_payment(request(chain, asset).to_string()).unwrap()
}
fn sign(prepared: &str) -> String {
    static VAULT: OnceLock<String> = OnceLock::new();
    let vault = VAULT.get_or_init(|| {
        json_value(
            wallet_core::restore_wallet(
                "test test test test test test test test test test test junk".into(),
                vec![11; 32],
            )
            .unwrap(),
        )["vault_hex"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let parsed: Value = serde_json::from_str(prepared).unwrap();
    wallet_core::sign_payment(
        vault.clone(),
        vec![11; 32],
        prepared.into(),
        parsed["approval_digest"].as_str().unwrap().into(),
    )
    .unwrap()
}
fn block() -> String {
    format!("0x{}", "12".repeat(32))
}
fn receipt(signed: &str) -> Value {
    let signed: Value = serde_json::from_str(signed).unwrap();
    let actual = json_value(
        wallet_core::inspect_signed(signed["raw_transaction"].as_str().unwrap().into()).unwrap(),
    );
    let mut logs = Vec::new();
    if !matches!(
        signed["asset"].as_str().unwrap(),
        "ETH" | "AVAX" | "POL" | "BNB"
    ) {
        let amount = U256::from_dec_str(signed["amount_units"].as_str().unwrap()).unwrap();
        logs.push(json!({"address":actual["transaction_to"],
            "topics":["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
            format!("0x{:0>64}",&FROM[2..].to_ascii_lowercase()),format!("0x{:0>64}",&TO[2..].to_ascii_lowercase())],
            "data":format!("0x{amount:064x}"),"transactionHash":signed["hash"],"blockHash":block(),
            "blockNumber":"0x64","logIndex":"0x0","removed":false}));
    }
    json!({"transactionHash":signed["hash"],"blockHash":block(),"blockNumber":"0x64",
        "from":FROM,"to":actual["transaction_to"],"status":"0x1","gasUsed":"0x5208",
        "effectiveGasPrice":"0x59682f00","type":"0x2","contractAddress":null,"logs":logs,
        "l1Fee":"0x3e8","operatorFee":"0x64"})
}
fn journal() -> (tempfile::TempDir, Journal, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("mainnet.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let journal = Journal::open(path.clone()).unwrap();
    (dir, journal, path)
}

#[test]
fn fixed_registry_has_only_reviewed_mainnets_and_native_circle_contracts() {
    let registry = json_value(wallet_core::asset_registry());
    let networks = registry["networks"].as_array().unwrap();
    assert_eq!(networks.len(), 7);
    assert_eq!(networks[0]["chain_id"], 1);
    assert_eq!(networks[1]["chain_id"], 8453);
    assert_eq!(networks[2]["chain_id"], 10);
    assert_eq!(networks[3]["chain_id"], 42161);
    assert_eq!(networks[2]["network"], "OP Mainnet");
    assert_eq!(networks[3]["network"], "Arbitrum One");
    // These literals are independently copied from Circle's native-USDC
    // registry, not derived from the implementation constants under test.
    assert_eq!(
        networks[2]["assets"][1]["contract"],
        "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85"
    );
    assert_eq!(
        networks[3]["assets"][1]["contract"],
        "0xaf88d065e77c8cC2239327C5EDb3A432268e5831"
    );
    assert_eq!(
        networks[1]["broadcast_url"],
        "https://mainnet-sequencer.base.org"
    );
    for (index, chain) in [1, 8453, 10, 42161].into_iter().enumerate() {
        assert_eq!(
            networks[index]["assets"].as_array().unwrap().len(),
            if chain == 1 {
                4
            } else if chain == 8453 {
                3
            } else {
                2
            }
        );
        for (asset_index, asset) in [crypto::Asset::Eth, crypto::Asset::Usdc, crypto::Asset::Eurc]
            .into_iter()
            .enumerate()
        {
            if !asset.supported_on(chain) {
                continue;
            }
            assert_eq!(
                networks[index]["assets"][asset_index]["decimals"],
                asset.decimals()
            );
            assert_eq!(
                networks[index]["assets"][asset_index]["contract"],
                json!(asset.contract(chain))
            );
        }
    }
    assert_ne!(
        networks[0]["assets"][1]["contract"],
        networks[1]["assets"][1]["contract"]
    );
}

#[test]
fn both_mainnets_sign_all_assets_with_distinct_chain_identity_and_contract() {
    for asset in ["ETH", "USDC", "EURC"] {
        let ethereum = json_value(sign(&prepare(1, asset)));
        let base = json_value(sign(&prepare(8453, asset)));
        assert_ne!(ethereum["hash"], base["hash"]);
        assert_eq!(ethereum["from"], base["from"]);
        for (chain, signed) in [(1, ethereum), (8453, base)] {
            let raw = signed["raw_transaction"].as_str().unwrap();
            assert!((raw.len() - 2) / 2 <= 1024);
            let inspected = json_value(wallet_core::inspect_signed(raw.into()).unwrap());
            assert_eq!(inspected["chain_id"], chain);
            assert_eq!(inspected["asset"], asset);
            let typed = match asset {
                "USDC" => crypto::Asset::Usdc,
                "EURC" => crypto::Asset::Eurc,
                _ => crypto::Asset::Eth,
            };
            assert_eq!(
                inspected["transaction_to"],
                typed.contract(chain).unwrap_or(TO)
            );
        }
    }
}

#[test]
fn all_testnets_other_mainnets_and_wrong_extra_fee_semantics_fail_closed() {
    for chain in [
        0,
        5,
        97,
        80002,
        42170,
        43113,
        11155111,
        11155420,
        421614,
        84532,
        u64::MAX,
    ] {
        assert!(wallet_core::prepare_payment(request(chain, "USDC").to_string()).is_err());
    }
    for field in ["l1_fee_estimate_wei", "operator_fee_estimate_wei"] {
        for chain in [1, 42161] {
            let mut no_separate_fees = request(chain, "ETH");
            no_separate_fees[field] = json!("1");
            assert!(wallet_core::prepare_payment(no_separate_fees.to_string()).is_err());
        }
        for invalid in [
            "-1",
            "01",
            "1.0",
            "0x1",
            "99999999999999999999999999999999999999999999999999999999999999999999999999999999",
        ] {
            for chain in [8453, 10] {
                let mut op_stack = request(chain, "ETH");
                op_stack[field] = json!(invalid);
                assert!(wallet_core::prepare_payment(op_stack.to_string()).is_err());
            }
        }
        for chain in [8453, 10] {
            let mut missing = request(chain, "ETH");
            missing.as_object_mut().unwrap().remove(field);
            assert!(wallet_core::prepare_payment(missing.to_string()).is_err());
        }
    }
}

#[test]
fn quotes_are_in_approval_and_saved_signature_but_not_in_transaction_bytes() {
    let first = json_value(prepare(8453, "USDC"));
    let mut changed = first.clone();
    changed["request"]["l1_fee_estimate_wei"] = json!("1001");
    assert!(wallet_core::validate_prepared_json(changed.to_string()).is_err());
    let signed = sign(&first.to_string());
    assert!(wallet_core::validate_signed_for_prepared(first.to_string(), signed.clone()).is_ok());
    for field in [
        "l1_fee_estimate_wei",
        "operator_fee_estimate_wei",
        "estimated_total_fee_wei",
        "fee_reserve_wei",
        "chain_id",
    ] {
        let mut tampered: Value = serde_json::from_str(&signed).unwrap();
        tampered[field] = if field == "chain_id" {
            json!(1)
        } else {
            json!("9999")
        };
        assert!(
            wallet_core::validate_signed_for_prepared(first.to_string(), tampered.to_string())
                .is_err()
        );
    }
    let execution = U256::from_dec_str(first["max_fee_wei"].as_str().unwrap()).unwrap();
    assert_eq!(
        first["estimated_total_fee_wei"],
        (execution + U256::from(1100)).to_string()
    );
    assert_eq!(
        first["fee_reserve_wei"],
        (execution + U256::from(2200)).to_string()
    );
}

#[test]
fn base_additional_fee_above_estimate_and_reserve_retains_financial_outcome() {
    let (_dir, journal, path) = journal();
    let prepared = prepare(8453, "USDC");
    let signed = sign(&prepared);
    journal.record_prepared(prepared).unwrap();
    journal.record_signed(signed.clone()).unwrap();
    let mut observed = receipt(&signed);
    // 0.01 ETH of additional actual L1 fee deliberately exceeds the quote and
    // execution cap. It must be recorded as observed expenditure, not rejected.
    observed["l1Fee"] = json!("0x2386f26fc10000");
    let actual = json_value(
        journal
            .observe_receipt(
                "public-mainnet-fixture".into(),
                observed.to_string(),
                block(),
                100,
            )
            .unwrap(),
    );
    assert_eq!(actual["execution"], "success");
    assert_eq!(actual["phase"], "finalized");
    assert_eq!(actual["additional_fee_exceeded_estimate"], true);
    assert_eq!(actual["actual_execution_fee_wei"], "31500000000000");
    assert_eq!(actual["actual_l1_fee_wei"], "10000000000000000");
    assert_eq!(actual["actual_operator_fee_wei"], "100");
    assert_eq!(actual["actual_fee_wei"], "10031500000000100");
    drop(journal);
    let reopened = Journal::open(path).unwrap();
    assert_eq!(
        json_value(
            reopened
                .get_payment("public-mainnet-fixture".into())
                .unwrap()
        ),
        actual
    );
    let shared = json_value(
        wallet_core::validate_receipt(signed, observed.to_string(), block(), 100).unwrap(),
    );
    for field in [
        "execution",
        "phase",
        "inclusion",
        "actual_fee_wei",
        "actual_execution_fee_wei",
        "actual_l1_fee_wei",
        "actual_operator_fee_wei",
        "additional_fee_exceeded_estimate",
        "receipt_fingerprint",
    ] {
        assert_eq!(actual[field], shared[field]);
    }
}

#[test]
fn base_receipt_requires_all_actual_fees_and_rejects_execution_cap_violation() {
    let signed = sign(&prepare(8453, "ETH"));
    for field in ["l1Fee", "operatorFee"] {
        let mut missing = receipt(&signed);
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            wallet_core::validate_receipt(signed.clone(), missing.to_string(), block(), 100)
                .is_err()
        );
    }
    let mut invalid = receipt(&signed);
    invalid["effectiveGasPrice"] = json!("0xb2d05e00");
    assert!(
        wallet_core::validate_receipt(signed.clone(), invalid.to_string(), block(), 100).is_err()
    );
    let mut mismatched: Value = serde_json::from_str(&signed).unwrap();
    mismatched["chain_id"] = json!(1);
    assert!(wallet_core::validate_receipt(
        mismatched.to_string(),
        receipt(&signed).to_string(),
        block(),
        100
    )
    .is_err());
    let ethereum = sign(&prepare(1, "ETH"));
    assert!(wallet_core::validate_receipt(
        ethereum.clone(),
        receipt(&ethereum).to_string(),
        block(),
        100
    )
    .is_err());
}

#[test]
fn reverted_base_payment_keeps_execution_and_additional_charges() {
    let signed = sign(&prepare(8453, "USDC"));
    let mut observed = receipt(&signed);
    observed["status"] = json!("0x0");
    observed["logs"] = json!([]);
    let checked = json_value(
        wallet_core::validate_receipt(signed, observed.to_string(), block(), 99).unwrap(),
    );
    assert_eq!(checked["execution"], "failed");
    assert_eq!(checked["phase"], "included");
    assert_eq!(checked["actual_fee_wei"], "31500000001100");
}

#[test]
fn network_journals_allow_same_nonce_but_never_mix_network_observations() {
    let (_dir, ethereum, path) = journal();
    ethereum.record_prepared(prepare(1, "USDC")).unwrap();
    assert!(ethereum.record_prepared(prepare(8453, "USDC")).is_err());
    let (_other, base, _) = journal();
    base.record_prepared(prepare(8453, "USDC")).unwrap();
    let mut snapshot = json!({"chain_id":8453,"address":FROM,"block_number":100,"block_hash":block(),"balances":{"ETH":"0","USDC":"0","EURC":"0"}});
    assert!(ethereum.record_balance(snapshot.to_string()).is_err());
    base.record_balance(snapshot.to_string()).unwrap();
    snapshot["chain_id"] = json!(1);
    ethereum.record_balance(snapshot.to_string()).unwrap();
    drop(ethereum);
    let reopened = Journal::open(path).unwrap();
    assert_eq!(
        json_value(reopened.latest_balance().unwrap())["chain_id"],
        1
    );
    assert!(reopened.record_prepared(prepare(8453, "EURC")).is_err());
}

#[test]
fn new_network_signatures_are_chain_bound_and_preserve_exact_token_units() {
    let mut hashes = std::collections::HashSet::new();
    for chain in [1, 8453, 10, 42161] {
        for asset in ["ETH", "USDC"] {
            let mut input = request(chain, asset);
            input["amount"] = json!(if asset == "ETH" {
                "0.123456789123456789"
            } else {
                "123456789.123456"
            });
            input["gas_limit"] = json!(80_000);
            let prepared = wallet_core::prepare_payment(input.to_string()).unwrap();
            let signed = sign(&prepared);
            let parsed = json_value(signed.clone());
            let inspected = json_value(
                wallet_core::inspect_signed(parsed["raw_transaction"].as_str().unwrap().into())
                    .unwrap(),
            );
            assert_eq!(inspected["chain_id"], chain);
            assert_eq!(inspected["asset"], asset);
            assert_eq!(
                inspected["amount_units"],
                if asset == "ETH" {
                    "123456789123456789"
                } else {
                    "123456789123456"
                }
            );
            assert_eq!(
                inspected["transaction_to"],
                if asset == "ETH" {
                    TO
                } else {
                    crypto::Asset::Usdc.contract(chain).unwrap()
                }
            );
            assert!(hashes.insert(inspected["hash"].as_str().unwrap().to_owned()));
            for other in [1, 8453, 10, 42161] {
                if chain == other {
                    continue;
                }
                let mut changed = parsed.clone();
                changed["chain_id"] = json!(other);
                assert!(wallet_core::validate_signed_for_prepared(
                    prepared.clone(),
                    changed.to_string()
                )
                .is_err());
            }
        }
    }
}

#[test]
fn unsupported_eurc_never_becomes_a_native_transfer_or_a_balance() {
    for chain in [10, 42161] {
        assert!(!crypto::Asset::Eurc.supported_on(chain));
        assert!(crypto::Asset::Eurc.contract(chain).is_none());
        assert!(wallet_core::prepare_payment(request(chain, "EURC").to_string()).is_err());
        for asset in ["ETH", "USDC"] {
            let mut invalid = request(chain, asset);
            invalid["to"] = json!(crypto::Asset::Usdc.contract(chain).unwrap());
            assert!(wallet_core::prepare_payment(invalid.to_string()).is_err());
        }
        let (_dir, journal, path) = journal();
        let valid = json!({"chain_id":chain,"address":FROM,"block_number":100,"block_hash":block(),"balances":{"ETH":"123","USDC":"10000"}});
        let recorded = journal.record_balance(valid.to_string()).unwrap();
        assert_eq!(
            json_value(recorded.clone())["balances"],
            json!({"ETH":"123","USDC":"10000"})
        );
        for unsupported in [json!("0"), json!("1"), Value::Null] {
            let mut invalid = valid.clone();
            invalid["balances"]["EURC"] = unsupported;
            assert!(journal.record_balance(invalid.to_string()).is_err());
        }
        drop(journal);
        assert_eq!(
            Journal::open(path).unwrap().latest_balance().unwrap(),
            recorded
        );
    }
    for chain in [1, 8453] {
        let (_dir, journal, _) = journal();
        let missing = json!({"chain_id":chain,"address":FROM,"block_number":100,"block_hash":block(),"balances":{"ETH":"0","USDC":"0"}});
        assert!(journal.record_balance(missing.to_string()).is_err());
    }
}

#[test]
fn op_mainnet_preserves_reviewed_extra_fees_and_receipt_charges() {
    let prepared = prepare(10, "USDC");
    let parsed = json_value(prepared.clone());
    assert_eq!(parsed["estimated_total_fee_wei"], "160000000001100");
    assert_eq!(parsed["fee_reserve_wei"], "160000000002200");
    let signed = sign(&prepared);
    let observed = receipt(&signed);
    let checked = json_value(
        wallet_core::validate_receipt(signed.clone(), observed.to_string(), block(), 99).unwrap(),
    );
    assert_eq!(checked["execution"], "success");
    assert_eq!(checked["inclusion"], "included");
    assert_eq!(checked["actual_fee_wei"], "31500000001100");
    for fee in ["l1Fee", "operatorFee"] {
        let mut missing = observed.clone();
        missing.as_object_mut().unwrap().remove(fee);
        assert!(
            wallet_core::validate_receipt(signed.clone(), missing.to_string(), block(), 99)
                .is_err()
        );
    }
    let mut failed = observed.clone();
    failed["status"] = json!("0x0");
    failed["logs"] = json!([]);
    let checked = json_value(
        wallet_core::validate_receipt(signed, failed.to_string(), block(), 100).unwrap(),
    );
    assert_eq!(checked["execution"], "failed");
    assert_eq!(checked["actual_fee_wei"], "31500000001100");
}

#[test]
fn arbitrum_uses_the_gas_charge_once_and_rejects_separate_rollup_fees() {
    let mut input = request(42161, "USDC");
    input["gas_limit"] = json!(250_000);
    input["max_fee_per_gas"] = json!("20000000");
    input["max_priority_fee_per_gas"] = json!("0");
    let prepared = wallet_core::prepare_payment(input.to_string()).unwrap();
    let parsed = json_value(prepared.clone());
    assert_eq!(parsed["max_fee_wei"], "5000000000000");
    assert_eq!(parsed["max_fee_wei"], parsed["estimated_total_fee_wei"]);
    assert_eq!(parsed["max_fee_wei"], parsed["fee_reserve_wei"]);
    let signed = sign(&prepared);
    let mut observed = receipt(&signed);
    observed.as_object_mut().unwrap().remove("l1Fee");
    observed.as_object_mut().unwrap().remove("operatorFee");
    observed["gasUsed"] = json!("0x30d40"); // 200,000, including L1 posting gas.
    observed["effectiveGasPrice"] = json!("0x989680"); // 10,000,000 wei/gas.
    observed["gasUsedForL1"] = json!("0x10000"); // Informational subset, not an added charge.
    let checked = json_value(
        wallet_core::validate_receipt(signed.clone(), observed.to_string(), block(), 100).unwrap(),
    );
    assert_eq!(checked["actual_fee_wei"], "2000000000000");
    assert_eq!(checked["actual_l1_fee_wei"], "0");
    assert_eq!(checked["actual_operator_fee_wei"], "0");
    assert_eq!(checked["additional_fee_exceeded_estimate"], false);
    for field in ["l1Fee", "operatorFee"] {
        let mut duplicated = observed.clone();
        duplicated[field] = json!("0x1");
        assert!(wallet_core::validate_receipt(
            signed.clone(),
            duplicated.to_string(),
            block(),
            100
        )
        .is_err());
    }
    let mut tampered = json_value(signed.clone());
    tampered["l1_fee_estimate_wei"] = json!("100");
    tampered["estimated_total_fee_wei"] = json!("5000000000100");
    tampered["fee_reserve_wei"] = json!("5000000000200");
    assert!(wallet_core::validate_receipt(
        tampered.to_string(),
        observed.to_string(),
        block(),
        100
    )
    .is_err());
}

#[test]
fn new_chain_journals_retain_nonce_and_evidence_through_unknown_and_reorg_states() {
    for chain in [10, 42161] {
        let (_dir, journal, path) = journal();
        let prepared = prepare(chain, "USDC");
        let signed = sign(&prepared);
        let signed_value = json_value(signed.clone());
        journal.record_prepared(prepared.clone()).unwrap();
        journal.record_signed(signed.clone()).unwrap();
        journal
            .mark_submission("public-mainnet-fixture".into(), "unknown".into())
            .unwrap();
        let mut other = request(chain, "USDC");
        other["intent_id"] = json!("duplicate-nonce");
        assert!(journal
            .record_prepared(wallet_core::prepare_payment(other.to_string()).unwrap())
            .is_err());
        assert!(journal
            .cancel_prepared("public-mainnet-fixture".into())
            .is_err());
        let mut observed = receipt(&signed);
        if chain == 42161 {
            observed["l1Fee"] = json!("0x0");
            observed["operatorFee"] = json!("0x0");
        }
        let included = json_value(
            journal
                .observe_receipt(
                    "public-mainnet-fixture".into(),
                    observed.to_string(),
                    block(),
                    99,
                )
                .unwrap(),
        );
        assert_eq!(included["phase"], "included");
        let reorg = json_value(
            journal
                .mark_reorg("public-mainnet-fixture".into(), block())
                .unwrap(),
        );
        assert_eq!(reorg["phase"], "unknown");
        assert_eq!(reorg["hash"], signed_value["hash"]);
        assert_eq!(reorg["raw_transaction"], signed_value["raw_transaction"]);
        let finalized = journal
            .observe_receipt(
                "public-mainnet-fixture".into(),
                observed.to_string(),
                block(),
                100,
            )
            .unwrap();
        assert_eq!(json_value(finalized.clone())["phase"], "finalized");
        let events = journal
            .list_events("public-mainnet-fixture".into())
            .unwrap();
        drop(journal);
        let reopened = Journal::open(path).unwrap();
        assert_eq!(
            reopened
                .get_payment("public-mainnet-fixture".into())
                .unwrap(),
            finalized
        );
        assert_eq!(
            reopened
                .list_events("public-mainnet-fixture".into())
                .unwrap(),
            events
        );
        let other_chain = if chain == 10 { 42161 } else { 10 };
        assert!(reopened
            .record_prepared(prepare(other_chain, "USDC"))
            .is_err());
    }
}

#[test]
fn released_schema_two_journals_keep_signed_bytes_events_and_balances_without_migration() {
    for chain in [1, 8453] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-v2.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("fixtures/journal_schema_v2.sql"))
            .unwrap();
        let original_schema: String = connection.query_row("SELECT group_concat(sql, char(10)) FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)", [], |r| r.get(0)).unwrap();
        drop(connection);
        let journal = Journal::open(path.to_str().unwrap().into()).unwrap();
        let prepared = prepare(chain, "USDC");
        let signed = sign(&prepared);
        journal.record_prepared(prepared.clone()).unwrap();
        let record = journal.record_signed(signed.clone()).unwrap();
        let events = journal
            .list_events("public-mainnet-fixture".into())
            .unwrap();
        let balance = journal.record_balance(json!({"chain_id":chain,"address":FROM,"block_number":100,"block_hash":block(),"balances":{"ETH":"1","USDC":"10000","EURC":"0"}}).to_string()).unwrap();
        drop(journal);
        let reopened = Journal::open(path.to_str().unwrap().into()).unwrap();
        assert_eq!(
            reopened
                .get_payment("public-mainnet-fixture".into())
                .unwrap(),
            record
        );
        assert_eq!(
            reopened
                .list_events("public-mainnet-fixture".into())
                .unwrap(),
            events
        );
        assert_eq!(reopened.latest_balance().unwrap(), balance);
        assert_eq!(reopened.record_signed(signed.clone()).unwrap(), record);
        assert!(
            wallet_core::validate_signed_for_prepared(prepared.clone(), signed.clone()).is_ok()
        );
        let mut observed = receipt(&signed);
        if chain == 1 {
            observed["l1Fee"] = json!("0x0");
            observed["operatorFee"] = json!("0x0");
        }
        assert_eq!(
            json_value(
                reopened
                    .observe_receipt(
                        "public-mainnet-fixture".into(),
                        observed.to_string(),
                        block(),
                        100
                    )
                    .unwrap()
            )["phase"],
            "finalized"
        );
        for other_chain in [10, 42161] {
            assert!(reopened
                .record_prepared(prepare(other_chain, "USDC"))
                .is_err());
            assert!(reopened.record_balance(json!({"chain_id":other_chain,"address":FROM,"block_number":101,"block_hash":block(),"balances":{"ETH":"0","USDC":"0"}}).to_string()).is_err());
        }
        drop(reopened);
        let connection = rusqlite::Connection::open(path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        let schema: String = connection.query_row("SELECT group_concat(sql, char(10)) FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)", [], |r| r.get(0)).unwrap();
        assert_eq!(schema, original_schema);
        let saved: (String, String) = connection
            .query_row("SELECT prepared_json, signed_json FROM payments", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(json_value(saved.0), json_value(prepared));
        assert_eq!(json_value(saved.1), json_value(signed));
    }
}

#[test]
fn previous_testnet_journal_schema_is_preserved_and_not_migrated_into_mainnet() {
    let (_dir, journal, path) = journal();
    drop(journal);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);
    assert!(Journal::open(path.clone()).is_err());
    let connection = rusqlite::Connection::open(path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
}

#[test]
fn browser_kdf_matches_independent_libargon2_vector_and_rejects_invalid_parameters() {
    // Cross-checked with argon2-cffi's libargon2 implementation: Argon2id v19,
    // m=65536 KiB, t=3, p=1, output=32. This password is a public test fixture.
    let derived =
        wallet_core::derive_vault_key("public-password-fixture".into(), (0u8..16).collect())
            .unwrap();
    assert_eq!(
        hex::encode(derived),
        "63304e0f639a70751dfba7f0ab5ce77ffc4ae30bd8ce59df883cf5b9c3d3a7c8"
    );
    assert!(wallet_core::derive_vault_key("short".into(), vec![0; 16]).is_err());
    assert!(wallet_core::derive_vault_key("x".repeat(1025), vec![0; 16]).is_err());
    assert!(wallet_core::derive_vault_key("public-password-fixture".into(), vec![0; 15]).is_err());
    assert!(wallet_core::derive_vault_key("public-password-fixture".into(), vec![0; 17]).is_err());
}

#[test]
fn remaining_evm_registry_pins_native_assets_and_issuer_contracts() {
    let registry = json_value(wallet_core::asset_registry());
    let network = |chain: u64| {
        registry["networks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["chain_id"] == chain)
            .unwrap()
    };
    for (chain, native, count) in [(43114, "AVAX", 4), (137, "POL", 2), (56, "BNB", 1)] {
        assert_eq!(
            network(chain)["assets"][0],
            json!({"symbol":native,"decimals":18,"contract":null})
        );
        assert_eq!(network(chain)["assets"].as_array().unwrap().len(), count);
        assert!(!crypto::Asset::Eth.supported_on(chain));
        assert!(wallet_core::prepare_payment(request(chain, "ETH").to_string()).is_err());
    }
    assert_eq!(
        network(43114)["assets"][1]["contract"],
        "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E"
    );
    assert_eq!(
        network(43114)["assets"][2]["contract"],
        "0xC891EB4cbdEFf6e073e859e987815Ed1505c2ACD"
    );
    assert_eq!(
        network(43114)["assets"][3]["contract"],
        "0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7"
    );
    assert_eq!(
        network(137)["assets"][1]["contract"],
        "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"
    );
    assert_eq!(
        network(1)["assets"][3]["contract"],
        "0xdAC17F958D2ee523a2206206994597C13D831ec7"
    );
    for asset in ["USDC", "EURC", "USDT"] {
        assert!(wallet_core::prepare_payment(request(56, asset).to_string()).is_err());
    }
    for chain in [8453, 10, 42161, 137] {
        assert!(wallet_core::prepare_payment(request(chain, "USDT").to_string()).is_err());
    }
}

#[test]
fn all_new_assets_preserve_exact_units_and_bind_review_to_native_currency() {
    for (chain, assets) in [
        (1, vec!["USDT"]),
        (43114, vec!["AVAX", "USDC", "EURC", "USDT"]),
        (137, vec!["POL", "USDC"]),
        (56, vec!["BNB"]),
    ] {
        for asset in assets {
            let native = matches!(asset, "AVAX" | "POL" | "BNB");
            let mut input = request(chain, asset);
            input["amount"] = json!(if native {
                "0.123456789123456789"
            } else {
                "123456789.123456"
            });
            let prepared = wallet_core::prepare_payment(input.to_string()).unwrap();
            let signed = sign(&prepared);
            let parsed = json_value(signed.clone());
            let inspected = json_value(
                wallet_core::inspect_signed(parsed["raw_transaction"].as_str().unwrap().into())
                    .unwrap(),
            );
            assert_eq!(inspected["asset"], asset);
            assert_eq!(inspected["chain_id"], chain);
            assert_eq!(
                inspected["amount_units"],
                if native {
                    "123456789123456789"
                } else {
                    "123456789123456"
                }
            );
            assert_eq!(inspected["data"].as_str().unwrap() == "0x", native);
            assert!(
                wallet_core::validate_signed_for_prepared(prepared.clone(), signed.clone()).is_ok()
            );
            let (_dir, journal, path) = journal();
            journal.record_prepared(prepared).unwrap();
            journal.record_signed(signed.clone()).unwrap();
            let mut proof = receipt(&signed);
            proof["l1Fee"] = json!("0x0");
            proof["operatorFee"] = json!("0x0");
            assert_eq!(
                json_value(
                    journal
                        .observe_receipt(
                            "public-mainnet-fixture".into(),
                            proof.to_string(),
                            block(),
                            100
                        )
                        .unwrap()
                )["phase"],
                "finalized"
            );
            drop(journal);
            let reopened = Journal::open(path).unwrap();
            assert_eq!(
                json_value(
                    reopened
                        .get_payment("public-mainnet-fixture".into())
                        .unwrap()
                )["asset"],
                asset
            );
            let mut wrong = parsed.clone();
            wrong["asset"] = json!("ETH");
            assert!(wallet_core::validate_signed_for_prepared(
                prepare(chain, asset),
                wrong.to_string()
            )
            .is_err());
        }
    }
}

#[test]
fn fee_policy_uses_native_chain_denominations_and_bnb_zero_base_envelope() {
    for (chain, native, gas_price, cap) in [
        (43114, "AVAX", "500000000000", "500000000000000000"),
        (137, "POL", "5000000000000", "1000000000000000000"),
        (56, "BNB", "10000000000", "10000000000000000"),
    ] {
        let mut input = request(chain, native);
        input["gas_limit"] = json!(200000);
        input["max_fee_per_gas"] = json!(gas_price);
        if chain == 56 {
            input["max_priority_fee_per_gas"] = json!(gas_price);
        }
        let prepared = json_value(wallet_core::prepare_payment(input.to_string()).unwrap());
        assert!(
            U256::from_dec_str(prepared["fee_reserve_wei"].as_str().unwrap()).unwrap()
                <= U256::from_dec_str(cap).unwrap()
        );
        let above = U256::from_dec_str(gas_price).unwrap() + U256::one();
        input["max_fee_per_gas"] = json!(above.to_string());
        if chain == 56 {
            input["max_priority_fee_per_gas"] = json!(above.to_string());
        }
        assert!(wallet_core::prepare_payment(input.to_string()).is_err());
        let mut extra = request(chain, native);
        extra["l1_fee_estimate_wei"] = json!("1");
        assert!(wallet_core::prepare_payment(extra.to_string()).is_err());
    }
    let mut unequal = request(56, "BNB");
    unequal["max_priority_fee_per_gas"] = json!("1");
    assert!(wallet_core::prepare_payment(unequal.to_string()).is_err());
    let mut polygon = request(137, "POL");
    polygon["gas_limit"] = json!(300000);
    polygon["max_fee_per_gas"] = json!("5000000000000");
    assert!(wallet_core::prepare_payment(polygon.to_string()).is_err()); // 1.5 POL exceeds 1 POL policy.
}

#[test]
fn native_balance_snapshots_reject_eth_aliases_missing_assets_and_nulls() {
    for (chain, native, balances) in [
        (
            43114,
            "AVAX",
            json!({"AVAX":"123456789123456789","USDC":"10000","EURC":"0","USDT":"1"}),
        ),
        (137, "POL", json!({"POL":"123","USDC":"10000"})),
        (56, "BNB", json!({"BNB":"123"})),
    ] {
        let (_dir, journal, path) = journal();
        let snapshot = json!({"chain_id":chain,"address":FROM,"block_number":100,"block_hash":block(),"balances":balances});
        let recorded = journal.record_balance(snapshot.to_string()).unwrap();
        for replacement in [Value::Null, json!("-1"), json!("01")] {
            let mut wrong = snapshot.clone();
            wrong["balances"][native] = replacement;
            assert!(journal.record_balance(wrong.to_string()).is_err());
        }
        let mut wrong = snapshot.clone();
        wrong["balances"]["ETH"] = json!("0");
        assert!(journal.record_balance(wrong.to_string()).is_err());
        let mut wrong = snapshot.clone();
        wrong["balances"].as_object_mut().unwrap().remove(native);
        assert!(journal.record_balance(wrong.to_string()).is_err());
        drop(journal);
        assert_eq!(
            Journal::open(path.clone())
                .unwrap()
                .latest_balance()
                .unwrap(),
            recorded
        );
        let db = rusqlite::Connection::open(path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            4
        );
    }
}

#[test]
fn schema_three_journals_preserve_old_allowlist_and_unknown_ethereum_usdt() {
    for chain in [1, 8453, 10, 42161] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-v3.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(include_str!("fixtures/journal_schema_v3.sql"))
            .unwrap();
        let schema: String = db.query_row("SELECT group_concat(sql, char(10)) FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)", [], |r| r.get(0)).unwrap();
        drop(db);
        let journal = Journal::open(path.to_str().unwrap().into()).unwrap();
        let prepared = prepare(chain, "USDC");
        let signed = sign(&prepared);
        journal.record_prepared(prepared).unwrap();
        let stored = journal.record_signed(signed.clone()).unwrap();
        let mut assets = json!({"ETH":"1","USDC":"10000"});
        if matches!(chain, 1 | 8453) {
            assets["EURC"] = json!("0");
        }
        let saved = journal.record_balance(json!({"chain_id":chain,"address":FROM,"block_number":100,"block_hash":block(),"balances":assets}).to_string()).unwrap();
        drop(journal);
        let reopened = Journal::open(path.to_str().unwrap().into()).unwrap();
        assert_eq!(reopened.latest_balance().unwrap(), saved);
        assert_eq!(reopened.record_signed(signed).unwrap(), stored);
        for (other, native) in [(43114, "AVAX"), (137, "POL"), (56, "BNB")] {
            assert!(reopened.record_prepared(prepare(other, native)).is_err());
        }
        drop(reopened);
        let db = rusqlite::Connection::open(path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(db.query_row("SELECT group_concat(sql, char(10)) FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)", [], |r| r.get::<_,String>(0)).unwrap(), schema);
    }
}
