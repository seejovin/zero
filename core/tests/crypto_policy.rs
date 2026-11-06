//! Public, deliberately well-known test fixtures. Never fund these addresses.

use bip39::{Language, Mnemonic};
use proptest::prelude::*;
use serde_json::{json, Value};
use wallet_core::crypto::{
    self, Asset, InspectedPayment, PreparedPayment, SignedPayment, CHAIN_ID,
};

const PUBLIC_PHRASE: &str = "test test test test test test test test test test test junk";
const PUBLIC_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const RECIPIENT: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

fn request(asset: &str) -> Value {
    json!({
        "intent_id": "public-fixture-intent",
        "chain_id": CHAIN_ID,
        "from": PUBLIC_ADDRESS,
        "to": RECIPIENT,
        "asset": asset,
        "amount": "1.234567",
        "nonce": 7,
        "gas_limit": if asset == "ETH" { 21000 } else { 80000 },
        "max_fee_per_gas": "2000000000",
        "max_priority_fee_per_gas": "1000000000",
        "l1_fee_estimate_wei": "0", "operator_fee_estimate_wei": "0"
    })
}

fn prepare(asset: &str) -> PreparedPayment {
    serde_json::from_str(&crypto::prepare_payment(request(asset).to_string()).unwrap()).unwrap()
}

fn fixture_vault() -> String {
    let result: Value = serde_json::from_str(
        &crypto::restore_wallet(PUBLIC_PHRASE.into(), vec![0x42; 32]).unwrap(),
    )
    .unwrap();
    assert_eq!(result["address"], PUBLIC_ADDRESS);
    assert!(result.get("mnemonic").is_none());
    result["vault_hex"].as_str().unwrap().to_owned()
}

fn sign(prepared: &PreparedPayment) -> SignedPayment {
    serde_json::from_str(
        &crypto::sign_payment(
            fixture_vault(),
            vec![0x42; 32],
            serde_json::to_string(prepared).unwrap(),
            prepared.approval_digest.clone(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn restore_matches_independent_published_ethereum_fixture() {
    // Hardhat's first public development account, m/44'/60'/0'/0/0.
    let vault = fixture_vault();
    assert_eq!(
        crypto::reveal_mnemonic(vault, vec![0x42; 32]).unwrap(),
        PUBLIC_PHRASE
    );
    let result: Value = serde_json::from_str(
        &crypto::restore_wallet(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                .into(),
            vec![0x24; 32],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result["address"],
        "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
    );
}

#[test]
fn wallet_address_authenticates_the_vault_and_derives_its_identity() {
    let vault = fixture_vault();
    assert_eq!(
        crypto::wallet_address(vault.clone(), vec![0x42; 32]).unwrap(),
        PUBLIC_ADDRESS
    );
    assert!(crypto::wallet_address(vault.clone(), vec![0x41; 32]).is_err());
    let mut bytes = hex::decode(&vault[2..]).unwrap();
    bytes[20] ^= 1;
    assert!(crypto::wallet_address(format!("0x{}", hex::encode(bytes)), vec![0x42; 32]).is_err());
    assert!(crypto::wallet_address(vault, vec![0x42; 31]).is_err());
}

#[test]
fn generated_wallet_has_24_valid_words_and_restores_to_same_address() {
    let created: Value =
        serde_json::from_str(&crypto::generate_wallet(vec![0x31; 32]).unwrap()).unwrap();
    let phrase = created["mnemonic"].as_str().unwrap();
    let mnemonic = Mnemonic::parse_in(Language::English, phrase).unwrap();
    assert_eq!(mnemonic.word_count(), 24);
    let restored: Value =
        serde_json::from_str(&crypto::restore_wallet(phrase.into(), vec![0x32; 32]).unwrap())
            .unwrap();
    assert_eq!(created["address"], restored["address"]);
    assert_ne!(created["vault_hex"], restored["vault_hex"]);
    // Do not print even an ephemeral randomly generated phrase on assertion
    // failure. Fixed public test vectors elsewhere may use assert_eq! safely.
    assert!(
        crypto::reveal_mnemonic(
            created["vault_hex"].as_str().unwrap().into(),
            vec![0x31; 32]
        )
        .unwrap()
            == phrase
    );
}

#[test]
fn restore_accepts_all_standard_english_lengths() {
    for length in [16, 20, 24, 28, 32] {
        let phrase = Mnemonic::from_entropy(&vec![0x45; length])
            .unwrap()
            .to_string();
        let result: Value =
            serde_json::from_str(&crypto::restore_wallet(phrase.clone(), vec![0x46; 32]).unwrap())
                .unwrap();
        assert_eq!(
            crypto::reveal_mnemonic(result["vault_hex"].as_str().unwrap().into(), vec![0x46; 32])
                .unwrap(),
            phrase
        );
    }
}

#[test]
fn invalid_phrase_and_invalid_keys_fail_without_echoing_secret_input() {
    let secret_marker = "private-input-that-must-never-appear-in-errors";
    let error = crypto::restore_wallet(secret_marker.into(), vec![0x42; 32]).unwrap_err();
    assert!(!error.to_string().contains(secret_marker));
    assert!(crypto::restore_wallet("abandon ".repeat(12), vec![0x42; 32]).is_err());
    for length in [0, 1, 16, 31, 33, 64] {
        assert!(crypto::generate_wallet(vec![0; length]).is_err());
        assert!(crypto::restore_wallet(PUBLIC_PHRASE.into(), vec![0; length]).is_err());
    }
}

#[test]
fn vault_authentication_rejects_wrong_key_or_any_modified_byte() {
    let vault = fixture_vault();
    assert!(crypto::reveal_mnemonic(vault.clone(), vec![0x41; 32]).is_err());
    let bytes = hex::decode(&vault[2..]).unwrap();
    for position in 0..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[position] ^= 1;
        assert!(
            crypto::reveal_mnemonic(format!("0x{}", hex::encode(damaged)), vec![0x42; 32]).is_err()
        );
    }
    for length in 0..bytes.len() {
        assert!(crypto::reveal_mnemonic(
            format!("0x{}", hex::encode(&bytes[..length])),
            vec![0x42; 32]
        )
        .is_err());
    }
    assert!(crypto::reveal_mnemonic(format!("{vault}00"), vec![0x42; 32]).is_err());
}

#[test]
fn each_vault_encryption_uses_a_fresh_nonce() {
    let first = fixture_vault();
    let second = fixture_vault();
    assert_ne!(first, second);
    assert_eq!(
        crypto::reveal_mnemonic(first, vec![0x42; 32]).unwrap(),
        crypto::reveal_mnemonic(second, vec![0x42; 32]).unwrap()
    );
}

#[test]
fn all_three_assets_sign_only_the_reviewed_payment_and_recover_sender() {
    for asset in ["ETH", "USDC", "EURC"] {
        let prepared = prepare(asset);
        let signed = sign(&prepared);
        let inspected: InspectedPayment =
            serde_json::from_str(&crypto::inspect_signed(signed.raw_transaction.clone()).unwrap())
                .unwrap();
        assert_eq!(inspected.from, PUBLIC_ADDRESS);
        assert_eq!(inspected.to, RECIPIENT);
        assert_eq!(inspected.chain_id, CHAIN_ID);
        assert_eq!(inspected.amount, "1.234567");
        assert_eq!(inspected.nonce, 7);
        assert_eq!(inspected.hash, signed.hash);
        assert_eq!(
            inspected.unsigned_transaction,
            prepared.unsigned_transaction
        );
        assert_eq!(signed.approval_digest, prepared.approval_digest);
        assert_eq!(inspected.gas_limit, prepared.request.gas_limit);
        assert_eq!(inspected.max_fee_per_gas, "2000000000");
        assert_eq!(inspected.max_priority_fee_per_gas, "1000000000");
        match inspected.asset {
            Asset::Eth => {
                assert_eq!(inspected.transaction_to, RECIPIENT);
                assert_eq!(inspected.data, "0x");
                assert_eq!(inspected.amount_units, "1234567000000000000");
            }
            Asset::Usdc => {
                assert_eq!(inspected.transaction_to, crypto::USDC_CONTRACT);
                assert!(inspected.data.starts_with("0xa9059cbb"));
                assert_eq!(inspected.data.len(), 138);
                assert_eq!(inspected.amount_units, "1234567");
            }
            Asset::Eurc => {
                assert_eq!(inspected.transaction_to, crypto::EURC_CONTRACT);
                assert!(inspected.data.starts_with("0xa9059cbb"));
                assert_eq!(inspected.data.len(), 138);
                assert_eq!(inspected.amount_units, "1234567");
            }
            _ => panic!("This fixture only covers the original three assets"),
        }
    }
}

#[test]
fn repeating_an_identical_signature_returns_identical_bytes() {
    let prepared = prepare("USDC");
    let first = sign(&prepared);
    let second = sign(&prepared);
    assert_eq!(first.raw_transaction, second.raw_transaction);
    assert_eq!(first.hash, second.hash);
}

#[test]
fn changing_any_reviewed_field_is_rejected() {
    let prepared = prepare("USDC");
    let original = serde_json::to_value(&prepared).unwrap();
    let changes = [
        ("amount_units", json!("1234568")),
        ("max_fee_wei", json!("1")),
        ("unsigned_transaction", json!("0x02c0")),
        ("approval_digest", json!(format!("0x{}", "00".repeat(32)))),
        ("created_at", json!(prepared.created_at + 1)),
        ("expires_at", json!(prepared.expires_at + 1)),
    ];
    for (field, value) in changes {
        let mut changed = original.clone();
        changed[field] = value;
        assert!(
            crypto::validate_prepared(&changed.to_string()).is_err(),
            "{field}"
        );
    }
    let changes = [
        ("intent_id", json!("different-intent")),
        ("chain_id", json!(11_155_111)),
        ("from", json!(RECIPIENT)),
        ("to", json!(PUBLIC_ADDRESS)),
        ("asset", json!("EURC")),
        ("amount", json!("2.345678")),
        ("nonce", json!(8)),
        ("gas_limit", json!(80001)),
        ("max_fee_per_gas", json!("2000000001")),
        ("max_priority_fee_per_gas", json!("1000000001")),
    ];
    for (field, value) in changes {
        let mut changed = original.clone();
        changed["request"][field] = value;
        assert!(
            crypto::validate_prepared(&changed.to_string()).is_err(),
            "{field}"
        );
    }
    let mut changed = original;
    changed["unexpected"] = json!(true);
    assert!(crypto::validate_prepared(&changed.to_string()).is_err());
}

#[test]
fn wrong_approval_or_a_different_wallet_cannot_sign_review() {
    let prepared = prepare("ETH");
    let prepared_json = serde_json::to_string(&prepared).unwrap();
    assert!(crypto::sign_payment(
        fixture_vault(),
        vec![0x42; 32],
        prepared_json.clone(),
        "0x00".into()
    )
    .is_err());
    let restored: Value = serde_json::from_str(
        &crypto::restore_wallet(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(),
            vec![0x42; 32],
        ).unwrap()
    ).unwrap();
    assert!(crypto::sign_payment(
        restored["vault_hex"].as_str().unwrap().into(),
        vec![0x42; 32],
        prepared_json,
        prepared.approval_digest,
    )
    .is_err());
}

#[test]
fn amount_precision_is_exact_and_never_uses_floating_point() {
    let mut token = request("USDC");
    token["amount"] = json!("0.000001");
    let prepared: PreparedPayment =
        serde_json::from_str(&crypto::prepare_payment(token.to_string()).unwrap()).unwrap();
    assert_eq!(prepared.amount_units, "1");
    let mut eth = request("ETH");
    eth["amount"] = json!("0.000000000000000001");
    let prepared: PreparedPayment =
        serde_json::from_str(&crypto::prepare_payment(eth.to_string()).unwrap()).unwrap();
    assert_eq!(prepared.amount_units, "1");
    let mut eth = request("ETH");
    eth["amount"] = json!("9007199254740993.000000000000000001");
    let prepared: PreparedPayment =
        serde_json::from_str(&crypto::prepare_payment(eth.to_string()).unwrap()).unwrap();
    assert_eq!(prepared.amount_units, "9007199254740993000000000000000001");
    for amount in [
        "0",
        "0.000000",
        "1.0000001",
        "01",
        "1.",
        ".1",
        " 1",
        "1 ",
        "+1",
        "-1",
        "1e3",
        "1,000",
        "NaN",
        "∞",
        "1..0",
        "1_000",
        "０.１",
    ] {
        let mut token = request("USDC");
        token["amount"] = json!(amount);
        assert!(
            crypto::prepare_payment(token.to_string()).is_err(),
            "{amount}"
        );
    }
    let mut overflow = request("ETH");
    overflow["amount"] = json!("9".repeat(78));
    assert!(crypto::prepare_payment(overflow.to_string()).is_err());
}

#[test]
fn formatter_preserves_exact_small_and_large_values() {
    assert_eq!(crypto::format_amount("0".into(), 18).unwrap(), "0");
    assert_eq!(
        crypto::format_amount("1".into(), 18).unwrap(),
        "0.000000000000000001"
    );
    assert_eq!(crypto::format_amount("1000000".into(), 6).unwrap(), "1");
    assert_eq!(
        crypto::format_amount("1000001".into(), 6).unwrap(),
        "1.000001"
    );
    assert_eq!(
        crypto::format_amount("9007199254740993".into(), 0).unwrap(),
        "9007199254740993"
    );
    assert!(crypto::format_amount("01".into(), 6).is_err());
    assert!(crypto::format_amount("1".into(), 19).is_err());
    assert!(crypto::format_amount("9".repeat(79), 6).is_err());
}

#[test]
fn mainnet_unknown_fields_invalid_destinations_and_excess_fees_fail_closed() {
    let changes = [
        ("chain_id", json!(11_155_111)),
        ("asset", json!("USDT")),
        ("to", json!("0x0000000000000000000000000000000000000000")),
        ("to", json!(crypto::USDC_CONTRACT)),
        ("to", json!(crypto::EURC_CONTRACT)),
        ("to", json!("0x70997970c51812dc3A010C7d01b50e0d17dc79C8")),
        ("nonce", json!(u64::MAX)),
        ("gas_limit", json!(20999)),
        ("gas_limit", json!(300001)),
        ("max_fee_per_gas", json!("500000000001")),
        ("max_fee_per_gas", json!("0")),
        ("max_fee_per_gas", json!("02000000000")),
        ("max_priority_fee_per_gas", json!("2000000001")),
        ("intent_id", json!("../path")),
        ("intent_id", json!("")),
    ];
    for (field, value) in changes {
        let mut changed = request("ETH");
        changed[field] = value;
        assert!(
            crypto::prepare_payment(changed.to_string()).is_err(),
            "{field}"
        );
    }
    let mut changed = request("USDC");
    changed["gas_limit"] = json!(21000);
    assert!(crypto::prepare_payment(changed.to_string()).is_err());
    let mut changed = request("ETH");
    changed["gas_limit"] = json!(300000);
    changed["max_fee_per_gas"] = json!("500000000000");
    assert!(crypto::prepare_payment(changed.to_string()).is_err());
    let mut changed = request("ETH");
    changed["data"] = json!("0x095ea7b3");
    assert!(crypto::prepare_payment(changed.to_string()).is_err());
    let duplicate = request("ETH").to_string().replacen("{", "{\"nonce\":8,", 1);
    assert!(crypto::prepare_payment(duplicate).is_err());
}

#[test]
fn gas_limit_reserves_the_eip7623_calldata_floor() {
    // This transfer's 68 ABI bytes contain 41 zero bytes and 27 nonzero bytes:
    // 21000 + 10 * (41 + 4*27) = 22490. The old intrinsic cost is only 21596.
    let mut changed = request("USDC");
    changed["gas_limit"] = json!(22489);
    assert!(crypto::prepare_payment(changed.to_string()).is_err());
    changed["gas_limit"] = json!(22490);
    assert!(crypto::prepare_payment(changed.to_string()).is_ok());
}

#[test]
fn lowercase_addresses_and_trailing_fraction_zeroes_normalize_before_approval() {
    let mut input = request("USDC");
    input["from"] = json!(PUBLIC_ADDRESS.to_lowercase());
    input["to"] = json!(RECIPIENT.to_lowercase());
    input["amount"] = json!("1.230000");
    let prepared: PreparedPayment =
        serde_json::from_str(&crypto::prepare_payment(input.to_string()).unwrap()).unwrap();
    assert_eq!(prepared.request.from, PUBLIC_ADDRESS);
    assert_eq!(prepared.request.to, RECIPIENT);
    assert_eq!(prepared.request.amount, "1.23");
    assert_eq!(prepared.amount_units, "1230000");
    assert!(crypto::validate_prepared(&serde_json::to_string(&prepared).unwrap()).is_ok());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_transaction_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..1400)) {
        let _ = crypto::inspect_signed(format!("0x{}", hex::encode(bytes)));
    }

    #[test]
    fn arbitrary_unicode_inputs_fail_without_panics(input in ".{0,300}") {
        let _ = crypto::prepare_payment(input.clone());
        let _ = crypto::inspect_signed(input.clone());
        let _ = crypto::normalize_address(&input);
        let _ = crypto::format_amount(input, 6);
    }

    #[test]
    fn integer_token_units_round_trip_exactly(units in 1u64..=u64::MAX) {
        let formatted = crypto::format_amount(units.to_string(), 6).unwrap();
        let mut input = request("EURC");
        input["amount"] = json!(formatted);
        let prepared: PreparedPayment = serde_json::from_str(
            &crypto::prepare_payment(input.to_string()).unwrap()
        ).unwrap();
        prop_assert_eq!(prepared.amount_units, units.to_string());
    }
}
