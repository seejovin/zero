//! Public, synthetic receipt evidence only. No key or transaction is broadcast.
use serde_json::{json, Value};
use wallet_core::{asset_registry, validate_activity};
const ACCOUNT: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const SENDER: &str = "0x1111111111111111111111111111111111111111";
const OTHER: &str = "0x2222222222222222222222222222222222222222";
const TRANSFER: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
fn hash(byte: &str) -> String {
    format!("0x{}", byte.repeat(32))
}
fn topic(address: &str) -> String {
    format!("0x{:0>64}", address[2..].to_lowercase())
}
fn fixture(chain: u64, asset: &str) -> (Value, Value, Value) {
    let registry: Value = serde_json::from_str(&asset_registry()).unwrap();
    let network = registry["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["chain_id"] == chain)
        .unwrap();
    let spec = network["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["symbol"] == asset)
        .unwrap();
    let destination = if matches!(asset, "ETH" | "AVAX" | "POL" | "BNB") {
        ACCOUNT.to_string()
    } else {
        spec["contract"].as_str().unwrap().to_string()
    };
    let tx = json!({"hash":hash("aa"),"chainId":format!("0x{chain:x}"),"blockHash":hash("11"),"blockNumber":"0x64","from":SENDER,"to":destination,"value":if matches!(asset, "ETH" | "AVAX" | "POL" | "BNB") {"0x2386f26fc10000"} else {"0x0"}});
    let logs = if matches!(asset, "ETH" | "AVAX" | "POL" | "BNB") {
        vec![]
    } else {
        vec![
            json!({"address":destination,"topics":[TRANSFER,topic(SENDER),topic(ACCOUNT)],"data":format!("0x{:064x}",10_000u64),"transactionHash":hash("aa"),"blockHash":hash("11"),"blockNumber":"0x64","logIndex":"0x9","removed":false}),
        ]
    };
    let receipt = json!({"transactionHash":hash("aa"),"blockHash":hash("11"),"blockNumber":"0x64","from":SENDER,"to":destination,"status":"0x1","logs":logs});
    let block = json!({"hash":hash("11"),"number":"0x64","timestamp":"0x6553f100","transactions":[hash("aa")]});
    (tx, receipt, block)
}
fn validate(
    chain: u64,
    tx: &Value,
    r: &Value,
    b: &Value,
    finalized: u64,
) -> Result<Value, wallet_core::WalletError> {
    validate_activity(
        chain,
        ACCOUNT.into(),
        tx.to_string(),
        r.to_string(),
        b.to_string(),
        finalized,
    )
    .map(|s| serde_json::from_str(&s).unwrap())
}
#[test]
fn exact_native_and_allowlisted_tokens_on_all_reviewed_networks() {
    for chain in [1, 8453, 10, 42161, 43114, 137, 56] {
        for asset in ["ETH", "USDC", "EURC", "USDT", "AVAX", "POL", "BNB"] {
            let typed: wallet_core::crypto::Asset = serde_json::from_value(json!(asset)).unwrap();
            if !typed.supported_on(chain) {
                continue;
            }
            let (tx, r, b) = fixture(chain, asset);
            let result = validate(chain, &tx, &r, &b, 99).unwrap();
            let a = &result["activities"][0];
            assert_eq!(result["activities"].as_array().unwrap().len(), 1);
            assert_eq!(a["asset"], asset);
            assert_eq!(a["chain_id"], chain);
            assert_eq!(a["direction"], "received");
            assert_eq!(a["amount"], "0.01");
            assert_eq!(a["inclusion"], "included");
            assert_eq!(
                validate(chain, &tx, &r, &b, 100).unwrap()["activities"][0]["inclusion"],
                "finalized"
            );
        }
    }
}

#[test]
fn new_network_activity_ignores_bridged_tokens_and_other_chain_contracts() {
    for (chain, bridged) in [
        (10, "0x7f5c764cbc14f9669b88837ca1490cca17c31607"),
        (42161, "0xff970a61a04b1ca14834a43f5de4533ebddb5cc8"),
    ] {
        let (tx, receipt, block) = fixture(chain, "USDC");
        for contract in [
            bridged,
            wallet_core::crypto::EURC_CONTRACT,
            wallet_core::crypto::BASE_EURC_CONTRACT,
            wallet_core::crypto::BASE_USDC_CONTRACT,
        ] {
            let mut wrong = receipt.clone();
            wrong["logs"][0]["address"] = json!(contract);
            assert_eq!(
                validate(chain, &tx, &wrong, &block, 100).unwrap()["activities"],
                json!([])
            );
        }
        for wrong_chain in [1, 8453, 10, 42161, 56, 11155420, 421614] {
            if wrong_chain != chain {
                assert!(validate(wrong_chain, &tx, &receipt, &block, 100).is_err());
            }
        }
        let mut removed = receipt.clone();
        removed["logs"][0]["removed"] = json!(true);
        assert!(validate(chain, &tx, &removed, &block, 100).is_err());
        let mut reorg = block.clone();
        reorg["hash"] = json!(hash("22"));
        assert!(validate(chain, &tx, &receipt, &reorg, 100).is_err());
    }
}
#[test]
fn wrong_token_or_participant_never_becomes_received() {
    let (tx, mut r, b) = fixture(8453, "USDC");
    r["logs"][0]["address"] = json!(OTHER);
    assert_eq!(
        validate(8453, &tx, &r, &b, 0).unwrap()["activities"],
        json!([])
    );
    let (tx, mut r, b) = fixture(8453, "USDC");
    r["logs"][0]["topics"][2] = json!(topic(OTHER));
    assert_eq!(
        validate(8453, &tx, &r, &b, 0).unwrap()["activities"],
        json!([])
    );
    let (tx, r, b) = fixture(1, "USDC");
    assert!(validate(8453, &tx, &r, &b, 0).is_err());
    assert!(validate(11155111, &tx, &r, &b, 0).is_err());
}
#[test]
fn all_receipt_transaction_and_block_links_are_required() {
    for path in ["transactionHash", "blockHash", "blockNumber", "from", "to"] {
        let (tx, mut r, b) = fixture(8453, "USDC");
        r[path] = json!(match path {
            "blockNumber" => "0x65".into(),
            "from" | "to" => OTHER.into(),
            _ => hash("ff"),
        });
        assert!(validate(8453, &tx, &r, &b, 100).is_err(), "{path}");
    }
    let (tx, r, mut b) = fixture(8453, "USDC");
    b["transactions"] = json!([hash("bb")]);
    assert!(validate(8453, &tx, &r, &b, 100).is_err());
    b["transactions"] = json!([hash("aa"), hash("aa")]);
    assert!(validate(8453, &tx, &r, &b, 100).is_err());
    b["transactions"] = json!([{ "hash":hash("aa"),"other":"ignored" }]);
    assert!(validate(8453, &tx, &r, &b, 100).is_ok());
}
#[test]
fn malformed_removed_duplicate_or_inconsistent_logs_are_rejected() {
    for field in [
        "transactionHash",
        "blockHash",
        "blockNumber",
        "removed",
        "topics",
        "data",
        "logIndex",
    ] {
        let (tx, mut r, b) = fixture(8453, "USDC");
        r["logs"][0][field] = match field {
            "blockNumber" => json!("0x65"),
            "removed" => json!(true),
            "topics" => json!([TRANSFER, topic(SENDER)]),
            "data" => json!("0x00"),
            "logIndex" => json!("0x00"),
            _ => json!(hash("ee")),
        };
        assert!(validate(8453, &tx, &r, &b, 0).is_err(), "{field}");
    }
    let (tx, mut r, b) = fixture(8453, "USDC");
    let duplicate = r["logs"][0].clone();
    r["logs"].as_array_mut().unwrap().push(duplicate);
    assert!(validate(8453, &tx, &r, &b, 0).is_err());
    let (tx, mut r, b) = fixture(8453, "USDC");
    r["logs"][0]["topics"][2] = json!(format!("0x1{}", &topic(ACCOUNT)[3..]));
    assert!(validate(8453, &tx, &r, &b, 0).is_err());
}
#[test]
fn reverted_transfer_zero_address_poisoning_and_mint_semantics() {
    let (tx, mut r, b) = fixture(8453, "USDC");
    r["status"] = json!("0x0");
    assert!(validate(8453, &tx, &r, &b, 0).is_err());
    r["logs"] = json!([]);
    assert_eq!(
        validate(8453, &tx, &r, &b, 0).unwrap()["activities"],
        json!([])
    );
    let (tx, mut r, b) = fixture(8453, "USDC");
    r["logs"][0]["data"] = json!(format!("0x{:064x}", 0));
    assert_eq!(
        validate(8453, &tx, &r, &b, 0).unwrap()["activities"],
        json!([])
    );
    r["logs"][0]["data"] = json!(format!("0x{:064x}", 1));
    r["logs"][0]["topics"][1] = json!(topic("0x0000000000000000000000000000000000000000"));
    assert_eq!(
        validate(8453, &tx, &r, &b, 0).unwrap()["activities"][0]["amount"],
        "0.000001"
    );
}
#[test]
fn self_and_sent_direction_and_stable_receipt_ordinal_identity() {
    let (tx, mut r, b) = fixture(8453, "USDC");
    r["logs"][0]["topics"][1] = json!(topic(ACCOUNT));
    let initial = validate(8453, &tx, &r, &b, 0).unwrap();
    assert_eq!(initial["activities"][0]["direction"], "self");
    r["logs"][0]["logIndex"] = json!("0xff");
    assert_eq!(
        validate(8453, &tx, &r, &b, 0).unwrap()["activities"][0]["id"],
        initial["activities"][0]["id"]
    );
    r["logs"][0]["topics"][2] = json!(topic(OTHER));
    assert_eq!(
        validate(8453, &tx, &r, &b, 0).unwrap()["activities"][0]["direction"],
        "sent"
    );
}
#[test]
fn duplicate_fields_and_oversized_input_are_rejected() {
    let (tx, r, b) = fixture(1, "USDC");
    let raw = r.to_string().replacen('{', "{\"status\":\"0x0\",", 1);
    assert!(validate_activity(1, ACCOUNT.into(), tx.to_string(), raw, b.to_string(), 0).is_err());
    assert!(validate_activity(
        1,
        ACCOUNT.into(),
        " ".repeat(1_048_577),
        r.to_string(),
        b.to_string(),
        0
    )
    .is_err());
}
