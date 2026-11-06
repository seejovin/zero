//! Public BIP39/Ledger fixtures only. No RPC access or network submission.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use wallet_core::{crypto, tron};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const OWNER: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const RECIPIENT: &str = "TDp4x8kCJ6aHz8wnqZgkaVQAm2ajH1og3L";
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn request(asset: &str) -> Value {
    let now = now();
    json!({"intent_id":"public-tron-test-intent","network":"tron-mainnet","from":OWNER,"to":RECIPIENT,"asset":asset,
        "amount":"0.010001","maximum":false,"balance_sun":"1000000000","token_balance_units":"10001",
        "ref_block_id":format!("{:016x}{}",80_000_000u64,"12".repeat(24)),"ref_block_number":80_000_000u64,
        "ref_block_timestamp_ms":now-1000,"observed_at_ms":now,"recipient_active":true,"staked_bandwidth":0,"free_bandwidth":0,"available_energy":0,
        "bandwidth_price_sun":"1000","energy_price_sun":"100","creation_fee_sun":"1000000","creation_bandwidth_fee_sun":"100000",
        "creation_bandwidth_rate":1,"max_fee_limit_sun":"15000000000","max_energy_factor":34000,"energy_used":if asset=="USDT"{65000}else{0},"energy_penalty":if asset=="USDT"{35000}else{0}})
}
fn vault() -> String {
    let v: Value =
        serde_json::from_str(&crypto::restore_wallet(PHRASE.into(), vec![0x42; 32]).unwrap())
            .unwrap();
    v["vault_hex"].as_str().unwrap().into()
}
fn prepare(asset: &str) -> tron::Prepared {
    tron::prepare(&request(asset).to_string()).unwrap()
}
fn sign(prepared: &tron::Prepared) -> tron::Signed {
    tron::sign(
        vault(),
        vec![0x42; 32],
        serde_json::to_string(prepared).unwrap(),
        prepared.approval_digest.clone(),
    )
    .unwrap()
}
fn evidence(p: &tron::Prepared, s: &tron::Signed, success: bool) -> Value {
    let height = 80_000_001u64;
    let blockid = format!("{height:016x}{}", "34".repeat(24));
    let stamp = p.created_at + 3000;
    let tx = json!({"txID":s.hash,"raw_data_hex":p.raw_data_hex,"signature":s.broadcast["signature"],"ret":[{"contractRet":if success{"SUCCESS"}else{"REVERT"}}]});
    let logs = if p.request.asset == "USDT" && success {
        vec![
            json!({"address":tron::address_hex(tron::USDT).unwrap()[2..],"topics":[
        "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        format!("{:0>64}",&tron::address_hex(OWNER).unwrap()[2..]),format!("{:0>64}",&tron::address_hex(RECIPIENT).unwrap()[2..])],"data":format!("{:064x}",10001)}),
        ]
    } else {
        vec![]
    };
    json!({"network":tron::NETWORK,"chain_id":tron::MAINNET_CHAIN_ID,"finality_source":"walletsolidity","transaction":tx,
        "info":{"id":s.hash,"fee":1000,"blockNumber":height,"blockTimeStamp":stamp,
            "receipt":{"net_fee":1000,"result":if success{"SUCCESS"}else{"REVERT"}},
            "contractResult":if p.request.asset=="USDT"&&success{vec![format!("{:064x}",1)]}else{vec![]},"log":logs},
        "block":{"blockID":blockid,"block_header":{"raw_data":{"number":height,"timestamp":stamp}},"transactions":[tx]},"solid_height":height+10})
}

#[test]
fn derivation_matches_published_ledger_tron_fixture_and_authenticates_vault() {
    // https://github.com/LedgerHQ/app-tron — its published test seed/address.
    let vault = vault();
    assert_eq!(
        tron::wallet_address(vault.clone(), vec![0x42; 32]).unwrap(),
        OWNER
    );
    assert!(tron::wallet_address(vault.clone(), vec![0x43; 32]).is_err());
    assert!(tron::wallet_address(vault, vec![0x42; 31]).is_err());
    assert_eq!(tron::DERIVATION_PATH, "m/44'/195'/0'/0/0");
}
#[test]
fn base58check_matches_primary_tron_address_vector() {
    // https://developers.tron.network/docs/account
    assert_eq!(
        tron::address_hex(RECIPIENT).unwrap(),
        "412a260a110bc7b03f19c40a0bd04ff2c5dcb57594"
    );
    assert_eq!(tron::normalize_address(OWNER).unwrap(), OWNER);
    for value in [
        "TDp4x8kCJ6aHz8wnqZgkaVQAm2ajH1og3M",
        "T9yD14Nj9j7xAB4dbGeiX9h8unkKHxuWwb",
        "0x2a260a110bc7b03f19c40a0bd04ff2c5dcb57594",
        " TDp4x8kCJ6aHz8wnqZgkaVQAm2ajH1og3L",
    ] {
        assert!(tron::normalize_address(value).is_err());
    }
}
#[test]
fn exact_six_decimal_units_and_official_usdt_contract_are_bound_to_wire() {
    for asset in ["TRX", "USDT"] {
        let p = prepare(asset);
        let signed = sign(&p);
        assert_eq!(p.amount_units, "10001");
        assert_eq!(
            p.hash,
            hex::encode(Sha256::digest(hex::decode(&p.raw_data_hex).unwrap()))
        );
        assert_eq!(signed.hash, p.hash);
        assert_eq!(
            signed.broadcast["signature"][0].as_str().unwrap().len(),
            130
        );
        assert!(tron::validate_signed(&serde_json::to_string(&signed).unwrap()).is_ok());
        let value = &signed.broadcast["raw_data"]["contract"][0]["parameter"]["value"];
        if asset == "USDT" {
            assert_eq!(
                value["contract_address"],
                tron::address_hex(tron::USDT).unwrap()
            );
            let expected = format!(
                "a9059cbb{:0>64}{:064x}",
                &tron::address_hex(RECIPIENT).unwrap()[2..],
                10001
            );
            assert_eq!(value["data"], expected);
            assert!(value.get("call_value").is_none());
            assert_eq!(p.fee_limit_sun, "15840000");
        } else {
            assert_eq!(value["amount"], 10001);
            assert!(signed.broadcast["raw_data"].get("fee_limit").is_none());
        }
    }
}
#[test]
fn unsupported_assets_networks_self_and_contract_transfers_fail() {
    for asset in ["USDC", "EURC", "USDT.e", "TRC10"] {
        assert!(tron::prepare(&request(asset).to_string()).is_err());
    }
    for (field, value) in [
        ("network", json!("tron-shasta")),
        ("to", json!(OWNER)),
        ("to", json!(tron::USDT)),
    ] {
        let mut r = request("TRX");
        r[field] = value;
        assert!(tron::prepare(&r.to_string()).is_err());
    }
}
#[test]
fn invalid_precision_signs_or_exponents_are_never_rounded() {
    for amount in [
        "0",
        "0.0000001",
        "1.0000001",
        "1e6",
        "-1",
        "+1",
        "01",
        "1.",
        " 1",
        "1,000",
    ] {
        let mut r = request("TRX");
        r["amount"] = json!(amount);
        assert!(tron::prepare(&r.to_string()).is_err(), "{amount}");
    }
    let mut r = request("TRX");
    r["amount"] = json!("0.000001");
    assert_eq!(tron::prepare(&r.to_string()).unwrap().amount_units, "1");
}
#[test]
fn token_max_transfers_every_integer_unit_including_sixth_decimal() {
    let mut r = request("USDT");
    r["maximum"] = json!(true);
    r["token_balance_units"] = json!("123456789012345678901234567890123456");
    let p = tron::prepare(&r.to_string()).unwrap();
    assert_eq!(p.amount_units, r["token_balance_units"].as_str().unwrap());
    assert!(p.request.maximum);
}
#[test]
fn native_max_reserves_reviewed_fees_and_never_spends_them() {
    let mut r = request("TRX");
    r["maximum"] = json!(true);
    let p = tron::prepare(&r.to_string()).unwrap();
    assert_eq!(
        p.amount_units.parse::<u64>().unwrap() + p.fee_reserve_units.parse::<u64>().unwrap(),
        1_000_000_000
    );
    assert!(
        p.fee_reserve_units.parse::<u64>().unwrap()
            >= p.estimated_fee_units.parse::<u64>().unwrap()
    );
}
#[test]
fn account_creation_uses_fixed_activation_and_staked_bandwidth_only() {
    let mut r = request("TRX");
    r["recipient_active"] = json!(false);
    r["free_bandwidth"] = json!(1_000_000);
    let p = tron::prepare(&r.to_string()).unwrap();
    assert_eq!(p.fee_reserve_units, "1100000");
    assert_eq!(p.estimated_fee_units, "1100000");
    r["staked_bandwidth"] = json!(1_000_000);
    let p = tron::prepare(&r.to_string()).unwrap();
    assert_eq!(p.estimated_fee_units, "1000000");
    assert_eq!(p.fee_reserve_units, "1100000");
}
#[test]
fn partial_bandwidth_quotas_cannot_be_added_together() {
    let p = prepare("TRX");
    let mut r = request("TRX");
    r["staked_bandwidth"] = json!(p.bandwidth_bytes - 1);
    r["free_bandwidth"] = json!(p.bandwidth_bytes - 1);
    let p = tron::prepare(&r.to_string()).unwrap();
    assert_eq!(
        p.estimated_fee_units,
        (p.bandwidth_bytes * 1000).to_string()
    );
    r["free_bandwidth"] = json!(p.bandwidth_bytes);
    assert_eq!(
        tron::prepare(&r.to_string()).unwrap().estimated_fee_units,
        "0"
    );
}
#[test]
fn energy_is_simulated_and_fee_limit_does_not_include_bandwidth() {
    let p = prepare("USDT");
    assert!(p.fee_reserve_units.parse::<u64>().unwrap() > p.fee_limit_sun.parse::<u64>().unwrap());
    let mut r = request("USDT");
    r["energy_used"] = json!(0);
    assert!(tron::prepare(&r.to_string()).is_err());
    r = request("USDT");
    r["energy_penalty"] = json!(65001);
    assert!(tron::prepare(&r.to_string()).is_err());
    r = request("USDT");
    r["available_energy"] = json!(1_000_000);
    let staked = tron::prepare(&r.to_string()).unwrap();
    assert_eq!(staked.fee_limit_sun, p.fee_limit_sun);
    assert_eq!(
        staked.estimated_fee_units,
        (staked.bandwidth_bytes * 1000).to_string()
    );
}
#[test]
fn stale_tapos_wrong_block_number_and_excessive_fee_policy_fail_closed() {
    let mut r = request("TRX");
    r["ref_block_number"] = json!(79_000_000);
    assert!(tron::prepare(&r.to_string()).is_err());
    r = request("TRX");
    r["ref_block_timestamp_ms"] = json!(now() - 120_000);
    assert!(tron::prepare(&r.to_string()).is_err());
    r = request("TRX");
    r["observed_at_ms"] = json!(now() - 200_000);
    r["ref_block_timestamp_ms"] = json!(now() - 201_000);
    assert!(tron::prepare(&r.to_string()).is_err());
    r = request("USDT");
    r["energy_price_sun"] = json!("100000");
    assert!(tron::prepare(&r.to_string()).is_err());
    r = request("USDT");
    r["max_fee_limit_sun"] = json!("1000");
    assert!(tron::prepare(&r.to_string()).is_err());
}
#[test]
fn insufficient_native_or_token_balance_prevents_signing() {
    for (asset, field, value) in [
        ("TRX", "balance_sun", "10001"),
        ("USDT", "balance_sun", "0"),
        ("USDT", "token_balance_units", "10000"),
    ] {
        let mut r = request(asset);
        r[field] = json!(value);
        assert!(tron::prepare(&r.to_string()).is_err());
    }
}
#[test]
fn reviewed_bytes_fee_metadata_and_approval_cannot_be_changed() {
    let p = prepare("USDT");
    let encoded = serde_json::to_value(&p).unwrap();
    for (field, value) in [
        ("amount_units", json!("10000")),
        ("raw_data_hex", json!("00")),
        ("fee_reserve_units", json!("0")),
        ("hash", json!("00".repeat(32))),
    ] {
        let mut changed = encoded.clone();
        changed[field] = value;
        assert!(tron::validate_prepared(&changed.to_string()).is_err());
    }
    assert!(tron::sign(
        vault(),
        vec![0x42; 32],
        encoded.to_string(),
        "wrong-approval".into()
    )
    .is_err());
}
#[test]
fn signed_wire_extra_fields_second_signatures_or_broadcast_mutation_are_rejected() {
    let signed = sign(&prepare("TRX"));
    let v = serde_json::to_value(signed).unwrap();
    let mut changed = v.clone();
    changed["raw_transaction"] = json!(format!("{}1801", v["raw_transaction"].as_str().unwrap()));
    assert!(tron::validate_signed(&changed.to_string()).is_err());
    let mut changed = v.clone();
    changed["broadcast"]["raw_data"]["contract"][0]["parameter"]["value"]["amount"] = json!(2);
    assert!(tron::validate_signed(&changed.to_string()).is_err());
    let mut changed = v.clone();
    changed["broadcast"]["signature"]
        .as_array_mut()
        .unwrap()
        .push(json!("00".repeat(65)));
    assert!(tron::validate_signed(&changed.to_string()).is_err());
    let mut changed = v;
    changed["from"] = json!(RECIPIENT);
    assert!(tron::validate_signed(&changed.to_string()).is_err());
}
#[test]
fn exact_solidified_trx_and_usdt_receipts_resolve_success() {
    for asset in ["TRX", "USDT"] {
        let p = prepare(asset);
        let s = sign(&p);
        let e = evidence(&p, &s, true);
        let observed = tron::validate_receipt(
            &serde_json::to_string(&p).unwrap(),
            &serde_json::to_string(&s).unwrap(),
            &e.to_string(),
        )
        .unwrap();
        assert_eq!(observed.state, "finalized");
        assert_eq!(observed.actual_fee_units, "1000");
        let activity = tron::validate_activity(RECIPIENT, &e.to_string()).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].direction, "received");
        assert_eq!(activity[0].amount, "0.010001");
    }
}
#[test]
fn reverted_transaction_records_fee_without_inventing_token_delivery() {
    let p = prepare("USDT");
    let s = sign(&p);
    let e = evidence(&p, &s, false);
    let observed = tron::validate_receipt(
        &serde_json::to_string(&p).unwrap(),
        &serde_json::to_string(&s).unwrap(),
        &e.to_string(),
    )
    .unwrap();
    assert_eq!(observed.state, "reverted");
    assert_eq!(observed.actual_fee_units, "1000");
    assert!(tron::validate_activity(RECIPIENT, &e.to_string())
        .unwrap()
        .is_empty());
}
#[test]
fn mismatched_network_finality_block_receipt_or_signature_never_credit_a_deposit() {
    let p = prepare("USDT");
    let s = sign(&p);
    let e = evidence(&p, &s, true);
    for mutation in 0..7 {
        let mut changed = e.clone();
        match mutation {
            0 => changed["chain_id"] = json!("0x94a9059e"),
            1 => changed["solid_height"] = json!(1),
            2 => changed["info"]["id"] = json!("00".repeat(32)),
            3 => changed["block"]["transactions"] = json!([]),
            4 => changed["info"]["blockNumber"] = json!(80_000_002),
            5 => changed["transaction"]["signature"] = json!(["00".repeat(65)]),
            _ => changed["finality_source"] = json!("wallet"),
        }
        assert!(tron::validate_receipt(
            &serde_json::to_string(&p).unwrap(),
            &serde_json::to_string(&s).unwrap(),
            &changed.to_string()
        )
        .is_err());
    }
}
#[test]
fn token_return_value_amount_contract_and_recipient_all_must_match() {
    let p = prepare("USDT");
    let s = sign(&p);
    let e = evidence(&p, &s, true);
    for mutation in 0..4 {
        let mut changed = e.clone();
        match mutation {
            0 => changed["info"]["contractResult"] = json!([format!("{:064x}", 0)]),
            1 => changed["info"]["log"][0]["data"] = json!(format!("{:064x}", 10000)),
            2 => changed["info"]["log"][0]["address"] = json!("11".repeat(20)),
            _ => {
                changed["info"]["log"][0]["topics"][2] =
                    json!(format!("{:0>64}", &tron::address_hex(OWNER).unwrap()[2..]))
            }
        }
        assert!(tron::validate_receipt(
            &serde_json::to_string(&p).unwrap(),
            &serde_json::to_string(&s).unwrap(),
            &changed.to_string()
        )
        .is_err());
    }
}
#[test]
fn malformed_duplicated_json_fields_do_not_cross_the_policy_boundary() {
    let text = request("TRX").to_string();
    let doubled = text.replacen('{', "{\"network\":\"tron-shasta\",", 1);
    assert!(tron::prepare(&doubled).is_err());
    let p = prepare("TRX");
    let mut changed = serde_json::to_value(p).unwrap();
    changed["unreviewed_extra_field"] = json!(true);
    assert!(tron::validate_prepared(&changed.to_string()).is_err());
}
