//! Offline, deliberately public fixtures. NEVER fund these addresses. No RPC or
//! broadcast is performed by this test suite.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use wallet_core::{crypto, solana};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
// Independent Python hashlib/HMAC-SHA512 + cryptography Ed25519 calculation of
// BIP39 + m/44'/501'/0'/0', included as a reproducible public golden fixture.
const OWNER: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const RECIPIENT: &str = "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB";

fn vault() -> String {
    let data: Value =
        serde_json::from_str(&crypto::restore_wallet(PHRASE.into(), vec![0x42; 32]).unwrap())
            .unwrap();
    data["vault_hex"].as_str().unwrap().to_string()
}
fn request(asset: &str) -> Value {
    let token = asset != "SOL";
    json!({
        "intent_id":"public-solana-fixture-0001", "network":solana::NETWORK,
        "genesis_hash":solana::GENESIS_HASH, "from":OWNER,"to":RECIPIENT,
        "asset":asset,"amount":if token {"0.010001"} else {"0.010000001"},
        "recent_blockhash":bs58::encode([19u8;32]).into_string(),
        "last_valid_block_height":250_000_000,"context_slot":300_000_000,
        "fee_lamports":"5000","rent_lamports":if token {"2039280"} else {"0"},
        "create_destination_ata":token,
        "source_accounts":if token {json!([{"address":solana::token_account(OWNER,asset).unwrap(),"amount_units":"10001"}])} else {json!([])},
        "compute_unit_limit":if token {200000} else {10000},"compute_unit_price_micro_lamports":"0"
    })
}
fn prepare(asset: &str) -> solana::Prepared {
    solana::prepare(&request(asset).to_string()).unwrap()
}
fn sign(prepared: &solana::Prepared) -> solana::Signed {
    solana::sign(
        &vault(),
        &[0x42; 32],
        &serde_json::to_string(prepared).unwrap(),
        &prepared.approval_digest,
    )
    .unwrap()
}
fn key_list(prepared: &solana::Prepared) -> Vec<String> {
    let message = BASE64.decode(&prepared.unsigned_transaction).unwrap();
    let count = message[3] as usize;
    (0..count)
        .map(|i| bs58::encode(&message[4 + i * 32..4 + (i + 1) * 32]).into_string())
        .collect()
}
fn token(index: usize, mint: &str, owner: &str, amount: u64) -> Value {
    json!({"accountIndex":index,"mint":mint,"owner":owner,"programId":solana::TOKEN_PROGRAM,
        "uiTokenAmount":{"amount":amount.to_string(),"decimals":6,"uiAmount":Value::Null,"uiAmountString":"unused"}})
}
fn evidence(prepared: &solana::Prepared, signed: &solana::Signed, success: bool) -> Value {
    let keys = key_list(prepared);
    let idx = |key: &str| keys.iter().position(|x| x == key).unwrap();
    let mut pre = vec![0u64; keys.len()];
    let mut post = vec![0u64; keys.len()];
    pre[idx(OWNER)] = 1_000_000_000;
    post[idx(OWNER)] = 1_000_000_000 - 5000;
    let mut pre_token = vec![];
    let mut post_token = vec![];
    if prepared.request.asset == solana::Asset::Sol {
        if success {
            post[idx(OWNER)] -= 10_000_001;
            post[idx(RECIPIENT)] = 10_000_001;
        }
    } else {
        let source = prepared.request.source_accounts[0].address.as_str();
        let dest = prepared.destination_token_account.as_deref().unwrap();
        let mint = prepared.request.asset.mint().unwrap();
        pre[idx(source)] = 2_039_280;
        post[idx(source)] = 2_039_280;
        pre_token.push(token(idx(source), mint, OWNER, 10001));
        post_token.push(token(
            idx(source),
            mint,
            OWNER,
            if success { 0 } else { 10001 },
        ));
        if success {
            post[idx(OWNER)] -= 2_039_280;
            post[idx(dest)] = 2_039_280;
            post_token.push(token(idx(dest), mint, RECIPIENT, 10001));
        }
    }
    let err = if success {
        Value::Null
    } else {
        json!({"InstructionError":[2,"InsufficientFunds"]})
    };
    json!({"network":solana::NETWORK,"genesis_hash":solana::GENESIS_HASH,"requested_signature":signed.hash,"canonical_block_slot":300_000_001,
        "transaction":{"slot":300_000_001,"transaction":[signed.raw_transaction,"base64"],"version":"legacy",
            "meta":{"err":err,"fee":5000,"preBalances":pre,"postBalances":post,"preTokenBalances":pre_token,"postTokenBalances":post_token,"innerInstructions":[]}},
        "signature_status":{"slot":300_000_001,"confirmations":Value::Null,"err":err,"confirmationStatus":"finalized"},
        "canonical_block":{"blockhash":bs58::encode([20u8;32]).into_string(),"previousBlockhash":bs58::encode([18u8;32]).into_string(),"parentSlot":300_000_000,"signatures":[signed.hash]},
        "finalized_slot":300_000_002})
}

#[test]
fn bip39_sol_derivation_matches_independent_fixture_and_authenticates_vault() {
    let vault = vault();
    assert_eq!(solana::address(&vault, &[0x42; 32]).unwrap(), OWNER);
    assert!(solana::address(&vault, &[0x43; 32]).is_err());
    assert_eq!(solana::DERIVATION_PATH, "m/44'/501'/0'/0'");
    // SLIP-0010 published test vector, root seed 000102...0f / m/0'/1'.
    let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
    assert_eq!(
        hex::encode(slip10_ed25519::derive_ed25519_private_key(&seed, &[0, 1])),
        "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2"
    );
}

#[test]
fn canonical_asset_mints_and_exact_all_decimal_units_are_bound() {
    for symbol in ["SOL", "USDC", "EURC", "USDT"] {
        let prepared = prepare(symbol);
        assert_eq!(
            prepared.amount_units,
            if symbol == "SOL" { "10000001" } else { "10001" }
        );
        let signed = sign(&prepared);
        assert!(solana::validate_signed(
            &serde_json::to_string(&prepared).unwrap(),
            &serde_json::to_string(&signed).unwrap()
        )
        .is_ok());
        let wire = BASE64.decode(&signed.raw_transaction).unwrap();
        assert_eq!(wire[0], 1);
        assert!(wire.len() <= solana::MAX_PACKET_BYTES);
        assert_eq!(bs58::encode(&wire[1..65]).into_string(), signed.hash);
        assert_eq!(
            wire[65..],
            BASE64.decode(&prepared.unsigned_transaction).unwrap()
        );
        if symbol != "SOL" {
            assert!(
                key_list(&prepared).contains(&prepared.request.asset.mint().unwrap().to_string())
            );
        }
    }
    assert_eq!(solana::parse_amount("0.000000001", 9).unwrap(), 1);
    assert_eq!(
        solana::parse_amount("18446744073.709551615", 9).unwrap(),
        u64::MAX
    );
    for invalid in [
        "1.0000000001",
        "18446744073.709551616",
        "1e9",
        "-1",
        "+1",
        "01",
        " 1",
        "1.",
    ] {
        assert!(solana::parse_amount(invalid, 9).is_err(), "{invalid}");
    }
}

#[test]
fn blocks_wrong_network_dangerous_address_and_fee_manipulation() {
    for (field, value) in [
        ("network", json!("devnet")),
        ("genesis_hash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp")),
        ("to", json!(solana::SYSTEM_PROGRAM)),
        ("to", json!(OWNER)),
        ("to", json!("0x0000000000000000000000000000000000000001")),
        ("fee_lamports", json!("0")),
        ("fee_lamports", json!("50000001")),
        ("rent_lamports", json!("999")),
        ("compute_unit_limit", json!(1_400_001)),
        ("compute_unit_price_micro_lamports", json!("500000")),
        ("last_valid_block_height", json!(0)),
        ("context_slot", json!(0)),
        ("amount", json!("0")),
        ("asset", json!("FAKE")),
    ] {
        let mut request = request("SOL");
        request[field] = value;
        assert!(solana::prepare(&request.to_string()).is_err(), "{field}");
    }
    let mut request = request("USDC");
    request["source_accounts"][0]["amount_units"] = json!("10000");
    assert!(solana::prepare(&request.to_string()).is_err());
}

#[test]
fn review_mutations_and_any_signature_or_message_mutation_fail_closed() {
    let prepared = prepare("USDC");
    let signed = sign(&prepared);
    let prepared_json = serde_json::to_string(&prepared).unwrap();
    for field in [
        "amount_units",
        "fee_reserve_units",
        "unsigned_transaction",
        "approval_digest",
        "destination_token_account",
    ] {
        let mut changed = serde_json::to_value(&prepared).unwrap();
        changed[field] = json!("changed");
        assert!(
            solana::validate_prepared(&changed.to_string()).is_err(),
            "{field}"
        );
    }
    let mut changed = serde_json::to_value(&prepared).unwrap();
    changed["request"]["recent_blockhash"] = json!(bs58::encode([21u8; 32]).into_string());
    assert!(solana::validate_prepared(&changed.to_string()).is_err());
    assert!(solana::sign(&vault(), &[0x42; 32], &prepared_json, &"0".repeat(64)).is_err());
    let bytes = BASE64.decode(&signed.raw_transaction).unwrap();
    for offset in [0, 1, 32, 64, 65, bytes.len() - 1] {
        let mut corrupted = bytes.clone();
        corrupted[offset] ^= 1;
        let mut changed = signed.clone();
        changed.raw_transaction = BASE64.encode(corrupted);
        assert!(
            solana::validate_signed(&prepared_json, &serde_json::to_string(&changed).unwrap())
                .is_err()
        );
    }
    let mut wrong_hash = signed.clone();
    wrong_hash.hash = bs58::encode([1u8; 64]).into_string();
    assert!(
        solana::validate_signed(&prepared_json, &serde_json::to_string(&wrong_hash).unwrap())
            .is_err()
    );
}

#[test]
fn duplicate_or_overallocated_sources_cannot_make_an_inexact_max() {
    let mut request = request("USDC");
    let source = request["source_accounts"][0].clone();
    request["source_accounts"] = json!([source, source]);
    assert!(solana::prepare(&request.to_string()).is_err());
    request["source_accounts"][1]["address"] =
        json!("2KW2XRd9kwqet15Aha2oK3tYvd3nWbTFH1MBiRAv1BE1");
    assert!(solana::prepare(&request.to_string()).is_err());
    request["source_accounts"][0]["amount_units"] = json!("9999");
    request["source_accounts"][1]["amount_units"] = json!("2");
    let prepared = solana::prepare(&request.to_string()).unwrap();
    assert_eq!(prepared.amount_units, "10001");
}

#[test]
fn confirmed_and_failed_solana_receipts_require_real_exact_canonical_effects() {
    for asset in ["SOL", "USDC", "EURC", "USDT"] {
        let prepared = prepare(asset);
        let signed = sign(&prepared);
        let p = serde_json::to_string(&prepared).unwrap();
        let s = serde_json::to_string(&signed).unwrap();
        for success in [true, false] {
            let valid = evidence(&prepared, &signed, success);
            let observation = solana::validate_receipt(&p, &s, &valid.to_string()).unwrap();
            assert_eq!(observation.success, success);
            assert!(observation.finalized);
            assert_eq!(observation.fee_units, "5000");
            assert_eq!(
                observation.status,
                if success { "finalized" } else { "reverted" }
            );
        }
        let valid = evidence(&prepared, &signed, true);
        for mut changed in [valid.clone(), valid.clone(), valid.clone(), valid.clone()]
            .into_iter()
            .enumerate()
        {
            match changed.0 {
                0 => changed.1["canonical_block"]["signatures"] = json!([]),
                1 => changed.1["signature_status"]["slot"] = json!(300_000_002),
                2 => changed.1["finalized_slot"] = json!(300_000_000),
                _ => changed.1["transaction"]["meta"]["postBalances"][0] = json!(7),
            }
            assert!(solana::validate_receipt(&p, &s, &changed.1.to_string()).is_err());
        }
    }
}

#[test]
fn incoming_activity_uses_instruction_evidence_and_canonical_mints() {
    for asset in ["SOL", "USDC", "EURC", "USDT"] {
        let prepared = prepare(asset);
        let signed = sign(&prepared);
        let valid = evidence(&prepared, &signed, true);
        let received = solana::validate_activity(RECIPIENT, &valid.to_string()).unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].asset, asset);
        assert_eq!(received[0].direction, "received");
        assert_eq!(received[0].amount_units, prepared.amount_units);
        assert_eq!(received[0].hash, signed.hash);
        let sent = solana::validate_activity(OWNER, &valid.to_string()).unwrap();
        assert_eq!(sent[0].direction, "sent");
        assert!(solana::validate_activity(
            RECIPIENT,
            &evidence(&prepared, &signed, false).to_string()
        )
        .unwrap()
        .is_empty());
    }
}

#[test]
fn expiry_needs_finalized_height_history_lookup_and_invalid_blockhash() {
    let evidence = json!({"network":solana::NETWORK,"genesis_hash":solana::GENESIS_HASH,"recent_blockhash":bs58::encode([19u8;32]).into_string(),
        "last_valid_block_height":250_000_000,"context_slot":300_000_000,"finalized_context_slot":300_000_200,
        "current_finalized_block_height":250_000_001,"history_search_enabled":true,"signature_found":false,"blockhash_valid":false});
    assert!(solana::check_expiry(&evidence.to_string()).unwrap().expired);
    for (field, value) in [
        ("history_search_enabled", json!(false)),
        ("signature_found", json!(true)),
        ("blockhash_valid", json!(true)),
        ("current_finalized_block_height", json!(250_000_000)),
    ] {
        let mut changed = evidence.clone();
        changed[field] = value;
        assert!(!solana::check_expiry(&changed.to_string()).unwrap().expired);
    }
    let mut stale = evidence;
    stale["finalized_context_slot"] = json!(299_999_999);
    assert!(solana::check_expiry(&stale.to_string()).is_err());
}

#[test]
fn ambiguous_rpc_json_is_rejected_before_value_conversion() {
    let prepared = prepare("USDC");
    let signed = sign(&prepared);
    let valid = evidence(&prepared, &signed, true).to_string();
    let duplicate = valid.replace("\"fee\":5000", "\"fee\":9000,\"fee\":5000");
    assert_ne!(duplicate, valid);
    assert!(solana::validate_receipt(
        &serde_json::to_string(&prepared).unwrap(),
        &serde_json::to_string(&signed).unwrap(),
        &duplicate
    )
    .is_err());
    assert!(solana::validate_activity(RECIPIENT, &duplicate).is_err());
    let mut switched = evidence(&prepared, &signed, true);
    switched["requested_signature"] = json!(bs58::encode([1u8; 64]).into_string());
    assert!(solana::validate_activity(RECIPIENT, &switched.to_string()).is_err());
}

#[test]
fn real_finality_survives_a_fee_or_rent_quote_discrepancy() {
    for symbol in ["SOL", "USDC"] {
        let prepared = prepare(symbol);
        let signed = sign(&prepared);
        let keys = key_list(&prepared);
        let payer = keys.iter().position(|key| key == OWNER).unwrap();
        let mut actual = evidence(&prepared, &signed, true);
        actual["transaction"]["meta"]["fee"] = json!(6000);
        let before = actual["transaction"]["meta"]["postBalances"][payer]
            .as_u64()
            .unwrap();
        actual["transaction"]["meta"]["postBalances"][payer] = json!(before - 1000);
        if symbol == "USDC" {
            let destination = keys
                .iter()
                .position(|key| Some(key.as_str()) == prepared.destination_token_account.as_deref())
                .unwrap();
            let before = actual["transaction"]["meta"]["postBalances"][payer]
                .as_u64()
                .unwrap();
            actual["transaction"]["meta"]["postBalances"][payer] = json!(before - 10);
            actual["transaction"]["meta"]["postBalances"][destination] = json!(2_039_290);
        }
        let observed = solana::validate_receipt(
            &serde_json::to_string(&prepared).unwrap(),
            &serde_json::to_string(&signed).unwrap(),
            &actual.to_string(),
        )
        .unwrap();
        assert!(observed.success && observed.finalized && observed.cost_exceeds_review);
        assert_eq!(observed.fee_units, "6000");
        assert_eq!(
            observed.account_rent_units,
            if symbol == "SOL" { "0" } else { "2039290" }
        );
        assert_eq!(
            observed.total_cost_units,
            if symbol == "SOL" { "6000" } else { "2045290" }
        );
    }
}

#[test]
fn a_caught_failed_cpi_cannot_manufacture_a_received_transfer() {
    let prepared = prepare("USDC");
    let signed = sign(&prepared);
    let mut actual = evidence(&prepared, &signed, true);
    let keys = key_list(&prepared);
    let index = |address: &str| keys.iter().position(|key| key == address).unwrap();
    let mut data = vec![12];
    data.extend_from_slice(&10001u64.to_le_bytes());
    data.push(6);
    actual["transaction"]["meta"]["innerInstructions"] = json!([{"index":2,"instructions":[{
        "programIdIndex":index(solana::TOKEN_PROGRAM),"stackHeight":2,
        "accounts":[index(&prepared.request.source_accounts[0].address),index(solana::USDC_MINT),index(prepared.destination_token_account.as_deref().unwrap()),index(OWNER)],
        "data":bs58::encode(data).into_string()
    }]}]);
    actual["transaction"]["meta"]["logMessages"] = json!([
        format!("Program {} invoke [1]", solana::ASSOCIATED_TOKEN_PROGRAM),
        format!("Program {} invoke [2]", solana::TOKEN_PROGRAM),
        format!(
            "Program {} failed: custom program error: 0x1",
            solana::TOKEN_PROGRAM
        ),
        format!("Program {} success", solana::ASSOCIATED_TOKEN_PROGRAM),
        format!("Program {} invoke [1]", solana::TOKEN_PROGRAM),
        format!("Program {} success", solana::TOKEN_PROGRAM)
    ]);
    let rows = solana::validate_activity(RECIPIENT, &actual.to_string()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_index, 3000);
    // Even a contradictory success log cannot overstate the actual credit.
    actual["transaction"]["meta"]["logMessages"][2] =
        json!(format!("Program {} success", solana::TOKEN_PROGRAM));
    assert!(solana::validate_activity(RECIPIENT, &actual.to_string())
        .unwrap()
        .is_empty());
    // Absent logs do not turn an attempted inner instruction into a transfer.
    actual["transaction"]["meta"]["logMessages"] = Value::Null;
    let rows = solana::validate_activity(RECIPIENT, &actual.to_string()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_index, 3000);
}
