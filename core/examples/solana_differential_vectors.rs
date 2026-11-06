//! Public, unfunded, offline vectors for independent @solana/web3.js checks.
//! No endpoint is contacted and the synthetic blockhash cannot be submitted.
use serde_json::{json, Value};
use wallet_core::{crypto, solana};

fn main() {
    let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let key = [0x42u8; 32];
    let vault: Value =
        serde_json::from_str(&crypto::restore_wallet(phrase.into(), key.to_vec()).unwrap())
            .unwrap();
    let vault_hex = vault["vault_hex"].as_str().unwrap();
    let owner = solana::address(vault_hex, &key).unwrap();
    let recipient = "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB";
    let mut vectors = Vec::new();
    for (asset, create_ata, multi_source) in [
        ("SOL", false, false),
        ("USDC", true, false),
        ("USDC", false, true),
        ("EURC", true, false),
        ("USDT", false, false),
    ] {
        let source = if asset == "SOL" {
            Vec::new()
        } else if multi_source {
            vec![
                json!({"address":solana::token_account(&owner,asset).unwrap(),"amount_units":"10000"}),
                json!({"address":"2KW2XRd9kwqet15Aha2oK3tYvd3nWbTFH1MBiRAv1BE1","amount_units":"1"}),
            ]
        } else {
            vec![
                json!({"address":solana::token_account(&owner,asset).unwrap(),"amount_units":"10001"}),
            ]
        };
        let request = json!({"intent_id":format!("public-solana-differential-{}",vectors.len()),
            "network":solana::NETWORK,"genesis_hash":solana::GENESIS_HASH,"from":owner,"to":recipient,
            "asset":asset,"amount":if asset=="SOL" {"0.010000001"} else {"0.010001"},
            "recent_blockhash":bs58::encode([19u8;32]).into_string(),"last_valid_block_height":250_000_000,
            "context_slot":300_000_000,"fee_lamports":"10000","rent_lamports":if create_ata {"2039280"} else {"0"},
            "create_destination_ata":create_ata,"source_accounts":source,"compute_unit_limit":200_000,
            "compute_unit_price_micro_lamports":"25000"});
        let prepared = solana::prepare(&request.to_string()).unwrap();
        let signed = solana::sign(
            vault_hex,
            &key,
            &serde_json::to_string(&prepared).unwrap(),
            &prepared.approval_digest,
        )
        .unwrap();
        vectors.push(json!({"prepared":prepared,"signed":signed}));
    }
    println!(
        "{}",
        json!({"fixture":"PUBLIC UNFUNDED OFFLINE ONLY","phrase":phrase,"derivation_path":solana::DERIVATION_PATH,"owner":owner,"vectors":vectors})
    );
}
