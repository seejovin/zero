//! Public deterministic test fixtures only. Never use this mnemonic for funds.
use serde_json::{json, Value};
use wallet_core::{crypto::Asset, prepare_payment, restore_wallet, sign_payment};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The standard public Anvil/Hardhat fixture, intentionally not a real secret.
    let phrase = "test test test test test test test test test test test junk";
    let key = vec![7u8; 32];
    let wallet: Value = serde_json::from_str(&restore_wallet(phrase.into(), key.clone())?)?;
    let cases = [
        ("ETH", "0.000000000000000001", 0u64, 21_000u64),
        ("ETH", "12.345678901234567890", 1, 21_000),
        ("USDC", "1.234567", 2, 100_000),
        ("EURC", "999999.000001", 3, 100_000),
        ("USDC", "0.000001", 4, 22_430),
        ("EURC", "0.000001", 5, 22_430),
        ("USDT", "0.000001", 6, 100_000),
        ("USDT", "999999.123456", 7, 100_000),
    ];
    let mut vectors = Vec::new();
    for chain_id in [1, 8453, 10, 42161, 43114, 137, 56] {
        for (asset, amount, nonce, gas_limit) in cases {
            let asset = if asset == "ETH" {
                match chain_id {
                    43114 => "AVAX",
                    137 => "POL",
                    56 => "BNB",
                    _ => "ETH",
                }
            } else {
                asset
            };
            let typed: Asset = serde_json::from_value(json!(asset))?;
            if !typed.supported_on(chain_id) {
                continue;
            }
            let request = json!({
                "intent_id": format!("fixture-{chain_id}-{asset}-{nonce}"),
                "chain_id": chain_id,
                "from": wallet["address"],
                "to": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
                "asset": asset,
                "amount": amount,
                "nonce": nonce,
                "gas_limit": gas_limit,
                "max_fee_per_gas": if chain_id == 56 { "1000000000" } else { "20000000000" },
                "max_priority_fee_per_gas": "1000000000",
                "l1_fee_estimate_wei": if matches!(chain_id, 8453 | 10) {"1000000"} else {"0"},
                "operator_fee_estimate_wei": if matches!(chain_id, 8453 | 10) {"10"} else {"0"}
            });
            let prepared_json = prepare_payment(request.to_string())?;
            let prepared: Value = serde_json::from_str(&prepared_json)?;
            let signed_json = sign_payment(
                wallet["vault_hex"].as_str().unwrap().into(),
                key.clone(),
                prepared_json,
                prepared["approval_digest"].as_str().unwrap().into(),
            )?;
            let signed: Value = serde_json::from_str(&signed_json)?;
            vectors.push(json!({"request":request,"prepared":prepared,"signed":signed}));
        }
    }
    println!("{}", serde_json::to_string(&vectors)?);
    Ok(())
}
