//! Closed pool queries: no arbitrary RPC, wallet filters, URLs or account data.
use crate::{private_payment, WalletError};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PoolRequest {
    Head {
        schema_version: u8,
        chain_id: u64,
    },
    Tip {
        schema_version: u8,
        chain_id: u64,
    },
    Block {
        schema_version: u8,
        chain_id: u64,
        number: u64,
    },
    Checkpoint {
        schema_version: u8,
        chain_id: u64,
        number: u64,
    },
    /// Complete approved pool logs for one canonical finalized block, derived
    /// by the adapter from every transaction receipt in that block. The fixed
    /// request cannot supply an address, topic, wallet or transaction filter.
    BlockLogs {
        schema_version: u8,
        chain_id: u64,
        number: u64,
    },
    Logs {
        schema_version: u8,
        chain_id: u64,
        from_block: u64,
        to_block: u64,
    },
    Root {
        schema_version: u8,
        chain_id: u64,
        tree_number: u64,
        root: String,
    },
    Receipt {
        schema_version: u8,
        chain_id: u64,
        transaction_hash: String,
    },
    Nullifiers {
        schema_version: u8,
        chain_id: u64,
        tree_number: u64,
        nullifiers: Vec<String>,
    },
}
const SAFE: u64 = 9_007_199_254_740_991;
fn invalid() -> WalletError {
    WalletError::Policy {
        reason: "Invalid public pool query".into(),
    }
}
pub fn validate(input: &str) -> Result<String, WalletError> {
    if input.len() > 4096 {
        return Err(invalid());
    }
    let q: PoolRequest = private_payment::decode(input)?;
    let (v, c) = match &q {
        PoolRequest::Head {
            schema_version,
            chain_id,
        }
        | PoolRequest::Tip {
            schema_version,
            chain_id,
        } => (*schema_version, *chain_id),
        PoolRequest::Block {
            schema_version,
            chain_id,
            number,
        }
        | PoolRequest::Checkpoint {
            schema_version,
            chain_id,
            number,
        }
        | PoolRequest::BlockLogs {
            schema_version,
            chain_id,
            number,
        } => {
            if *number > SAFE {
                return Err(invalid());
            }
            (*schema_version, *chain_id)
        }
        PoolRequest::Logs {
            schema_version,
            chain_id,
            from_block,
            to_block,
        } => {
            if to_block < from_block || *to_block > SAFE || to_block - from_block >= 2048 {
                return Err(invalid());
            }
            (*schema_version, *chain_id)
        }
        PoolRequest::Root {
            schema_version,
            chain_id,
            tree_number,
            root,
        } => {
            if *tree_number > 65535 {
                return Err(invalid());
            }
            private_payment::canonical_hash(root)?;
            (*schema_version, *chain_id)
        }
        PoolRequest::Receipt {
            schema_version,
            chain_id,
            transaction_hash,
        } => {
            private_payment::canonical_hash(transaction_hash)?;
            (*schema_version, *chain_id)
        }
        PoolRequest::Nullifiers {
            schema_version,
            chain_id,
            tree_number,
            nullifiers,
        } => {
            if *tree_number > 65535 || nullifiers.len() != 16 {
                return Err(invalid());
            }
            for n in nullifiers {
                private_payment::canonical_hash(n)?
            }
            if nullifiers
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != 16
            {
                return Err(invalid());
            }
            (*schema_version, *chain_id)
        }
    };
    if v != 1 || c != 42161 {
        return Err(invalid());
    }
    serde_json::to_string(&q).map_err(|_| invalid())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_wallet_filters_or_arbitrary_methods() {
        assert!(validate(r#"{"schema_version":1,"chain_id":42161,"kind":"head"}"#).is_ok());
        for x in [
            r#"{"schema_version":1,"chain_id":42161,"kind":"head","address":"0x123"}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"logs","from_block":0,"to_block":2048}"#,
            r#"{"schema_version":1,"chain_id":1,"kind":"head"}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"head","kind":"head"}"#,
        ] {
            assert!(validate(x).is_err())
        }
    }

    #[test]
    fn complete_block_logs_has_a_closed_wallet_independent_request_schema() {
        let valid = r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":12345}"#;
        let normalized = validate(valid).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&normalized).unwrap(),
            serde_json::from_str::<serde_json::Value>(valid).unwrap()
        );
        for invalid in [
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":12345,"address":"0x123"}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":12345,"topics":[]}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":12345,"transaction_hash":"0x123"}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":12345,"number":12346}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":-1}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":9007199254740992}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","number":"12345"}"#,
            r#"{"schema_version":1,"chain_id":1,"kind":"block_logs","number":12345}"#,
            r#"{"schema_version":2,"chain_id":42161,"kind":"block_logs","number":12345}"#,
            r#"{"schema_version":1,"chain_id":42161,"kind":"block_logs","from_block":1,"to_block":2}"#,
        ] {
            assert!(
                validate(invalid).is_err(),
                "unexpectedly accepted {invalid}"
            );
        }
    }
}
