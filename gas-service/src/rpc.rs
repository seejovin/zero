//! Bounded HTTPS JSON-RPC with independent-provider agreement, pinned deployment
//! identity and the selected RAILGUN V2 ABI. No customer-address lookup or generic
//! public RPC endpoint is exposed. Broadcast is called only after journal release.
use primitive_types::U256;
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use std::{
    collections::BTreeSet,
    io::Read,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use wallet_core::private_payment::{PrivateEnvelope, CANDIDATE_CHAIN_ID};

use crate::model::{bounded_hex, decimal, fixed_hex};
use crate::signer::GasQuote;
use crate::{
    AdmissionPolicy, ChainEvidenceVerifier, Error, PrivateExecution, ReceiptEvidence,
    ReorgEvidence, Result, TreasuryAnchor, VerifiedOperation,
};

const MAX_RPC_BYTES: usize = 1_048_576;
const MAX_BLOCK_TRANSACTIONS: usize = 4096;
const MAX_BLOCK_LOGS: usize = 8192;
const SESSION_DEADLINE: Duration = Duration::from_secs(20);

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodePin {
    pub address: String,
    pub keccak256: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoragePin {
    pub address: String,
    pub slot: String,
    pub value: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerificationKeyPin {
    pub inputs: u8,
    pub outputs: u8,
    pub encoded_key_keccak256: String,
}

/// A reviewed historical interval ends exclusively at `until_block`. Its start
/// is deployment_block or the preceding interval's end; gaps/overlaps cannot
/// be expressed. It qualifies read-only historical evidence, never admission.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalDeployment {
    pub until_block: u64,
    pub pool_code_keccak256: String,
    pub token_code_keccak256: String,
    /// Reviewed pre-deployment prefix only. The token pin must be Keccak256 of
    /// empty bytes and both providers must return exactly empty token code.
    /// This never applies to the current profile or authorizes admission.
    #[serde(default)]
    pub token_not_yet_deployed: bool,
    pub implementation_code_pins: Vec<CodePin>,
    pub proxy_storage_pins: Vec<StoragePin>,
}

/// Trusted operator configuration. Code/proxy/key pins must be qualified against
/// the deployed protocol; accepting this type does not approve those pins.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcConfig {
    pub primary_url: String,
    pub secondary_url: String,
    pub pool_code_keccak256: String,
    pub token_code_keccak256: String,
    pub implementation_code_pins: Vec<CodePin>,
    pub proxy_storage_pins: Vec<StoragePin>,
    #[serde(default)]
    pub historical_deployments: Vec<HistoricalDeployment>,
    pub verification_keys: Vec<VerificationKeyPin>,
    pub max_head_age_seconds: u64,
    pub deployment_block: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BlockHeader {
    pub number: u64,
    pub hash: String,
    pub parent_hash: String,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolLog {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
    pub block_hash: String,
    pub block_number: String,
    pub transaction_hash: String,
    pub transaction_index: String,
    pub log_index: String,
    pub removed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolReceipt {
    pub transaction_hash: String,
    pub block_hash: String,
    pub block_number: String,
    pub status: String,
    pub gas_used: String,
    pub effective_gas_price: String,
    pub logs: Vec<PoolLog>,
    #[serde(skip_serializing)]
    pub transaction_input: String,
}

/// Normalized complete-block evidence. Vendor-specific RPC receipt extensions
/// are not compared, but every transaction/index and every raw log is checked
/// before any fixed-pool filtering. These records are never persisted.
#[derive(PartialEq, Eq)]
struct BlockReceipt {
    transaction_hash: String,
    transaction_index: u64,
    status: u64,
    gas_used: U256,
    effective_gas_price: U256,
    logs: Vec<PoolLog>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    Nullifiers {
        schema_version: u8,
        chain_id: u64,
        tree_number: u64,
        nullifiers: Vec<String>,
    },
    Receipt {
        schema_version: u8,
        chain_id: u64,
        transaction_hash: String,
    },
}

impl PoolRequest {
    pub fn validate(&self) -> Result<()> {
        let (schema, chain) = match self {
            Self::Head {
                schema_version,
                chain_id,
            }
            | Self::Tip {
                schema_version,
                chain_id,
            }
            | Self::Block {
                schema_version,
                chain_id,
                ..
            }
            | Self::Checkpoint {
                schema_version,
                chain_id,
                ..
            }
            | Self::BlockLogs {
                schema_version,
                chain_id,
                ..
            }
            | Self::Logs {
                schema_version,
                chain_id,
                ..
            }
            | Self::Root {
                schema_version,
                chain_id,
                ..
            }
            | Self::Nullifiers {
                schema_version,
                chain_id,
                ..
            }
            | Self::Receipt {
                schema_version,
                chain_id,
                ..
            } => (*schema_version, *chain_id),
        };
        if schema != 1 || chain != CANDIDATE_CHAIN_ID {
            return Err(Error::InvalidEncoding);
        }
        match self {
            Self::Block { number, .. }
            | Self::Checkpoint { number, .. }
            | Self::BlockLogs { number, .. }
                if *number == 0 =>
            {
                return Err(Error::InvalidEncoding)
            }
            Self::Logs {
                from_block,
                to_block,
                ..
            } => {
                if *to_block < *from_block || to_block - from_block >= 2048 {
                    return Err(Error::InvalidEncoding);
                }
            }
            Self::Root {
                tree_number, root, ..
            } => {
                if *tree_number > 65535 {
                    return Err(Error::InvalidEncoding);
                }
                fixed_hex(root, 32)?;
            }
            Self::Nullifiers {
                tree_number,
                nullifiers,
                ..
            } => {
                if *tree_number > 65535 || nullifiers.len() != 16 {
                    return Err(Error::InvalidEncoding);
                }
                let mut seen = BTreeSet::new();
                for value in nullifiers {
                    fixed_hex(value, 32)?;
                    if !seen.insert(value) {
                        return Err(Error::InvalidEncoding);
                    }
                }
            }
            Self::Receipt {
                transaction_hash, ..
            } => fixed_hex(transaction_hash, 32)?,
            _ => {}
        }
        Ok(())
    }
}

pub struct ArbitrumRpc {
    config: RpcConfig,
    policy: AdmissionPolicy,
    client: Client,
    urls: [Url; 2],
    ids: AtomicU64,
}

struct Session<'a> {
    rpc: &'a ArbitrumRpc,
    started: Instant,
    calls: u16,
}

impl ArbitrumRpc {
    pub fn new(config: RpcConfig, policy: AdmissionPolicy) -> Result<Self> {
        Self::build(config, policy, false)
    }

    #[cfg(test)]
    pub(crate) fn local_fixture(config: RpcConfig, policy: AdmissionPolicy) -> Result<Self> {
        Self::build(config, policy, true)
    }

    pub fn validate_configuration(config: &RpcConfig, policy: &AdmissionPolicy) -> Result<()> {
        Self::validated_urls(config, policy, false).map(|_| ())
    }

    fn validated_urls(
        config: &RpcConfig,
        policy: &AdmissionPolicy,
        permit_loopback: bool,
    ) -> Result<[Url; 2]> {
        policy.validate()?;
        if policy.domain.chain_id != CANDIDATE_CHAIN_ID
            || config.max_head_age_seconds == 0
            || config.max_head_age_seconds > 300
            || config.deployment_block == 0
            || config.historical_deployments.len() > 64
            || config.verification_keys.is_empty()
            || config.verification_keys.len() > 256
        {
            return Err(Error::InvalidPolicy);
        }
        validate_deployment_pins(
            &config.pool_code_keccak256,
            &config.token_code_keccak256,
            &config.implementation_code_pins,
            &config.proxy_storage_pins,
        )?;
        let empty_code_hash = format!("0x{}", hex::encode(Keccak256::digest([])));
        if config.token_code_keccak256 == empty_code_hash {
            return Err(Error::InvalidPolicy);
        }
        let mut boundary = config.deployment_block;
        let mut token_deployed = false;
        for epoch in &config.historical_deployments {
            if epoch.until_block <= boundary {
                return Err(Error::InvalidPolicy);
            }
            if epoch.token_not_yet_deployed {
                if token_deployed || epoch.token_code_keccak256 != empty_code_hash {
                    return Err(Error::InvalidPolicy);
                }
            } else {
                if epoch.token_code_keccak256 == empty_code_hash {
                    return Err(Error::InvalidPolicy);
                }
                token_deployed = true;
            }
            validate_deployment_pins(
                &epoch.pool_code_keccak256,
                &epoch.token_code_keccak256,
                &epoch.implementation_code_pins,
                &epoch.proxy_storage_pins,
            )?;
            boundary = epoch.until_block;
        }
        let mut circuits = BTreeSet::new();
        for pin in &config.verification_keys {
            if !(1..=16).contains(&pin.inputs)
                || !(2..=16).contains(&pin.outputs)
                || !circuits.insert((pin.inputs, pin.outputs))
            {
                return Err(Error::InvalidPolicy);
            }
            fixed_hex(&pin.encoded_key_keccak256, 32)?;
        }
        let parse = |text: &str| -> Result<Url> {
            let url = Url::parse(text).map_err(|_| Error::InvalidPolicy)?;
            let loopback = url
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback());
            if url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || !(url.scheme() == "https"
                    || permit_loopback && loopback && url.scheme() == "http")
            {
                return Err(Error::InvalidPolicy);
            }
            Ok(url)
        };
        let urls = [parse(&config.primary_url)?, parse(&config.secondary_url)?];
        if urls[0] == urls[1] || !permit_loopback && urls[0].host_str() == urls[1].host_str() {
            return Err(Error::InvalidPolicy);
        }
        Ok(urls)
    }

    fn build(config: RpcConfig, policy: AdmissionPolicy, permit_loopback: bool) -> Result<Self> {
        let urls = Self::validated_urls(&config, &policy, permit_loopback)?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(6))
            .pool_max_idle_per_host(2)
            .build()
            .map_err(|_| Error::Rpc)?;
        Ok(Self {
            config,
            policy,
            client,
            urls,
            ids: AtomicU64::new(1),
        })
    }

    fn session(&self) -> Session<'_> {
        Session {
            rpc: self,
            started: Instant::now(),
            calls: 0,
        }
    }

    pub fn opening_treasury(&self) -> Result<TreasuryAnchor> {
        let mut s = self.session();
        s.check_chain()?;
        let block = s.agreed_head("finalized")?;
        s.check_current_deployment(&block)?;
        let balance = quantity(
            s.agree(
                "eth_getBalance",
                json!([self.policy.broadcaster_address, hex_quantity(block.number)]),
            )?
            .as_str()
            .ok_or(Error::Rpc)?,
        )?;
        s.recheck(&block)?;
        Ok(TreasuryAnchor {
            chain_id: self.policy.domain.chain_id,
            broadcaster_address: self.policy.broadcaster_address.clone(),
            balance_wei: balance.to_string(),
            block_number: block.number,
            block_hash: block.hash,
        })
    }

    pub fn canonical_block(&self, number: u64) -> Result<BlockHeader> {
        let mut s = self.session();
        s.check_chain()?;
        s.block(number)
    }

    pub fn quote(
        &self,
        envelope: &PrivateEnvelope,
        verified: &VerifiedOperation,
    ) -> Result<GasQuote> {
        let mut s = self.session();
        s.check_chain()?;
        let block = s.agreed_head("latest")?;
        s.require_fresh(&block)?;
        s.check_current_deployment(&block)?;
        s.check_private_state(envelope, verified, &block)?;
        let call = json!({"from":self.policy.broadcaster_address,"to":self.policy.domain.pool_contract,"value":"0x0","data":envelope.call_data});
        let gas = quantity(
            s.agree("eth_estimateGas", json!([call, hex_quantity(block.number)]))?
                .as_str()
                .ok_or(Error::Rpc)?,
        )?;
        if gas > self.policy.gas_limit.into() || gas < 21_000u64.into() {
            return Err(Error::CostExceeded);
        }
        let block_value = s.agree(
            "eth_getBlockByNumber",
            json!([hex_quantity(block.number), false]),
        )?;
        let base_fee = quantity(string(&block_value, "baseFeePerGas")?)?;
        let a = quantity(
            s.call(0, "eth_maxPriorityFeePerGas", json!([]))?
                .as_str()
                .ok_or(Error::Rpc)?,
        )?;
        let b = quantity(
            s.call(1, "eth_maxPriorityFeePerGas", json!([]))?
                .as_str()
                .ok_or(Error::Rpc)?,
        )?;
        let priority = std::cmp::max(a, b);
        let fee = base_fee
            .checked_mul(2u8.into())
            .and_then(|value| value.checked_add(priority))
            .ok_or(Error::Arithmetic)?;
        if fee
            .checked_mul(self.policy.gas_limit.into())
            .ok_or(Error::Arithmetic)?
            > decimal(&envelope.fee.max_native_fee_wei)?
        {
            return Err(Error::CostExceeded);
        }
        let nonce = quantity_u64(
            s.agree(
                "eth_getTransactionCount",
                json!([self.policy.broadcaster_address, "pending"]),
            )?
            .as_str()
            .ok_or(Error::Rpc)?,
        )?;
        s.recheck(&block)?;
        Ok(GasQuote {
            nonce,
            max_priority_fee_per_gas_wei: priority.to_string(),
            max_fee_per_gas_wei: fee.to_string(),
        })
    }

    pub fn pool_read(&self, request: &PoolRequest) -> Result<Value> {
        request.validate()?;
        let mut s = self.session();
        s.check_chain()?;
        let finalized = s.agreed_head("finalized")?;
        s.check_deployment(&finalized)?;
        let response = match request {
            PoolRequest::Head { .. } => {
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"head","block":finalized})
            }
            PoolRequest::Tip { .. } => {
                let block = s.agreed_head("latest")?;
                s.require_fresh(&block)?;
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"tip","block":block})
            }
            PoolRequest::Block { number, .. } => {
                let latest = s.agreed_head("latest")?;
                if *number > latest.number || *number < self.config.deployment_block {
                    return Err(Error::InvalidEvidence);
                }
                let block = s.block(*number)?;
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"block","block":block})
            }
            PoolRequest::Checkpoint { number, .. } => {
                if *number > finalized.number || *number < self.config.deployment_block {
                    return Err(Error::InvalidEvidence);
                }
                let block = s.block(*number)?;
                s.check_deployment(&block)?;
                let tree = U256::from_big_endian(&s.pool_getter("treeNumber()", &block)?);
                let next = U256::from_big_endian(&s.pool_getter("nextLeafIndex()", &block)?);
                let root = format!("0x{}", hex::encode(s.pool_getter("merkleRoot()", &block)?));
                if tree > 65_535u64.into() || next > 65_536u64.into() {
                    return Err(Error::InvalidEvidence);
                }
                fixed_hex(&root, 32)?;
                s.recheck(&block)?;
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"checkpoint","block":block,"tree_number":tree.low_u64(),"next_leaf_index":next.low_u64(),"merkle_root":root})
            }
            PoolRequest::BlockLogs { number, .. } => {
                if *number > finalized.number || *number < self.config.deployment_block {
                    return Err(Error::InvalidEvidence);
                }
                let (block, logs) = s.complete_block_logs(*number)?;
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"block_logs","block":block,"logs":logs})
            }
            PoolRequest::Root {
                tree_number, root, ..
            } => {
                let known = s.mapping(
                    "rootHistory(uint256,bytes32)",
                    *tree_number,
                    root,
                    &finalized,
                )?;
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"root","tree_number":tree_number,"root":root,"known":known,"block":finalized})
            }
            PoolRequest::Nullifiers {
                tree_number,
                nullifiers,
                ..
            } => {
                let mut spent = Vec::with_capacity(16);
                for nullifier in nullifiers {
                    spent.push(s.mapping(
                        "nullifiers(uint256,bytes32)",
                        *tree_number,
                        nullifier,
                        &finalized,
                    )?);
                }
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"nullifiers","tree_number":tree_number,"nullifiers":nullifiers,"spent":spent,"block":finalized})
            }
            PoolRequest::Logs {
                from_block,
                to_block,
                ..
            } => {
                if *to_block > finalized.number
                    || *from_block < self.config.deployment_block.saturating_sub(2047)
                {
                    return Err(Error::InvalidEvidence);
                }
                let count = to_block - from_block + 1;
                let aligned = count.is_power_of_two() && from_block.is_multiple_of(count);
                let truncated_tip = *to_block == finalized.number
                    && (1u64..=2048)
                        .filter(|n| n.is_power_of_two())
                        .any(|n| from_block.is_multiple_of(n) && count <= n);
                if !aligned && !truncated_tip {
                    return Err(Error::InvalidEncoding);
                }
                let block = s.block(*to_block)?;
                let topics = pool_topics();
                let values = s.agree("eth_getLogs",json!([{"address":self.policy.domain.pool_contract,"fromBlock":hex_quantity(*from_block),"toBlock":hex_quantity(*to_block),"topics":[topics]}]))?;
                let array = values.as_array().ok_or(Error::Rpc)?;
                if array.len() > 4096 {
                    return Err(Error::Rpc);
                }
                let mut logs = Vec::with_capacity(array.len());
                let mut seen = BTreeSet::new();
                for item in array {
                    let log = parse_log(item)?;
                    let number = quantity_u64(&log.block_number)?;
                    if number < *from_block
                        || number > *to_block
                        || log.address != self.policy.domain.pool_contract
                        || log.removed
                        || log.topics.len() != 1
                        || !topics.contains(&log.topics[0])
                        || !seen.insert((number, quantity_u64(&log.log_index)?))
                    {
                        return Err(Error::InvalidEvidence);
                    }
                    logs.push(log);
                }
                logs.sort_by_key(|log| {
                    (
                        quantity_u64(&log.block_number).unwrap_or_default(),
                        quantity_u64(&log.log_index).unwrap_or_default(),
                    )
                });
                s.recheck(&block)?;
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"logs","from_block":from_block,"to_block":to_block,"block_to_hash":block.hash,"logs":logs})
            }
            PoolRequest::Receipt {
                transaction_hash, ..
            } => {
                let receipt = s.pool_receipt(transaction_hash, false)?;
                if let Some(receipt) = &receipt {
                    let block = s.block(quantity_u64(&receipt.block_number)?)?;
                    if block.hash != receipt.block_hash {
                        return Err(Error::InvalidEvidence);
                    }
                }
                let transaction_input = receipt
                    .as_ref()
                    .map(|value| value.transaction_input.clone());
                json!({"schema_version":1,"chain_id":CANDIDATE_CHAIN_ID,"kind":"receipt","receipt":receipt,"finalized_block":finalized,"transaction_input":transaction_input})
            }
        };
        s.recheck(&finalized)?;
        if serde_json::to_vec(&response).map_err(|_| Error::Rpc)?.len() > 500_000 {
            return Err(Error::Rpc);
        }
        Ok(response)
    }

    pub fn operation_receipt(
        &self,
        id: &str,
        envelope: &PrivateEnvelope,
        verified: &VerifiedOperation,
        transaction_hash: &str,
    ) -> Result<Option<ReceiptEvidence>> {
        fixed_hex(id, 32)?;
        fixed_hex(transaction_hash, 32)?;
        let mut s = self.session();
        s.check_chain()?;
        let Some(receipt) = s.pool_receipt(transaction_hash, true)? else {
            return Ok(None);
        };
        let block = s.block(quantity_u64(&receipt.block_number)?)?;
        if block.hash != receipt.block_hash || receipt.transaction_input != envelope.call_data {
            return Err(Error::InvalidEvidence);
        }
        s.check_deployment(&block)?;
        let finalized = s.agreed_head("finalized")?;
        let execution = if receipt.status == "0x1" {
            match_private_logs(&receipt.logs, envelope, verified)?;
            PrivateExecution::Accepted {
                operation_digest: envelope.operation_digest.clone(),
                tree_number: verified.tree_number,
                nullifiers: envelope.nullifiers.clone(),
                output_commitments: envelope.output_commitments.clone(),
                fee_output_commitment: envelope.fee.fee_output_commitment.clone(),
            }
        } else if receipt.status == "0x0" && receipt.logs.is_empty() {
            PrivateExecution::Reverted
        } else {
            return Err(Error::InvalidEvidence);
        };
        s.recheck(&block)?;
        Ok(Some(ReceiptEvidence {
            request_id: id.into(),
            transaction_hash: transaction_hash.into(),
            chain_id: CANDIDATE_CHAIN_ID,
            block_number: block.number,
            block_hash: block.hash.clone(),
            canonical_block_hash: block.hash,
            finalized_height: finalized.number,
            gas_used: quantity_u64(&receipt.gas_used)?,
            effective_gas_price_wei: quantity(&receipt.effective_gas_price)?.to_string(),
            // Arbitrum Nitro gasUsed already incorporates the L1 gas component.
            // See official how-to-estimate-gas; never add gasUsedForL1 twice.
            additional_native_fee_wei: "0".into(),
            execution,
        }))
    }

    /// This is crate-private and can only be reached through OperatorWorker,
    /// after GasService commits a release ticket. It never constructs a new tx.
    pub(crate) fn submit_saved(&self, raw: &str, expected_hash: &str) -> Result<()> {
        bounded_hex(raw, 132_000)?;
        fixed_hex(expected_hash, 32)?;
        if format!(
            "0x{}",
            hex::encode(Keccak256::digest(bounded_hex(raw, 132_000)?))
        ) != expected_hash
        {
            return Err(Error::InvalidOuterTransaction);
        }
        let mut s = self.session();
        s.check_chain()?;
        // One endpoint per attempt. A timeout/error stays UNKNOWN; the worker
        // reconciles by expected hash before any identical-byte retry.
        let result = s.call(0, "eth_sendRawTransaction", json!([raw]))?;
        if result.as_str() != Some(expected_hash) {
            return Err(Error::InvalidEvidence);
        }
        Ok(())
    }
}

impl ChainEvidenceVerifier for ArbitrumRpc {
    fn verify_pending_exposure(&self, required_native_wei: &str) -> Result<()> {
        let required = decimal(required_native_wei)?;
        let mut s = self.session();
        s.check_chain()?;
        let block = s.agreed_head("latest")?;
        s.require_fresh(&block)?;
        s.check_current_deployment(&block)?;
        let balance = quantity(
            s.agree(
                "eth_getBalance",
                json!([self.policy.broadcaster_address, hex_quantity(block.number)]),
            )?
            .as_str()
            .ok_or(Error::Rpc)?,
        )?;
        if balance < required {
            return Err(Error::BudgetExceeded);
        }
        s.recheck(&block)
    }

    fn verify_treasury(&self, anchor: &TreasuryAnchor) -> Result<()> {
        anchor.validate(&self.policy)?;
        let mut s = self.session();
        s.check_chain()?;
        let finalized = s.agreed_head("finalized")?;
        if anchor.block_number > finalized.number {
            return Err(Error::InvalidEvidence);
        }
        let block = s.block(anchor.block_number)?;
        if block.hash != anchor.block_hash {
            return Err(Error::InvalidEvidence);
        }
        s.check_deployment(&block)?;
        let balance = quantity(
            s.agree(
                "eth_getBalance",
                json!([anchor.broadcaster_address, hex_quantity(block.number)]),
            )?
            .as_str()
            .ok_or(Error::Rpc)?,
        )?;
        if balance.to_string() != anchor.balance_wei {
            return Err(Error::InvalidEvidence);
        }
        s.recheck(&block)
    }
    fn verify_admission(
        &self,
        envelope: &PrivateEnvelope,
        verified: &VerifiedOperation,
    ) -> Result<()> {
        if envelope.domain != self.policy.domain || verified.additional_native_fee_cap_wei != "0" {
            return Err(Error::InvalidEvidence);
        }
        verified.validate(envelope, &self.policy)?;
        let mut s = self.session();
        s.check_chain()?;
        let block = s.agreed_head("latest")?;
        s.require_fresh(&block)?;
        s.check_current_deployment(&block)?;
        s.check_private_state(envelope, verified, &block)?;
        let balance = quantity(
            s.agree(
                "eth_getBalance",
                json!([self.policy.broadcaster_address, hex_quantity(block.number)]),
            )?
            .as_str()
            .ok_or(Error::Rpc)?,
        )?;
        if balance < decimal(&envelope.fee.max_native_fee_wei)? {
            return Err(Error::BudgetExceeded);
        }
        let call = json!({"from":self.policy.broadcaster_address,"to":self.policy.domain.pool_contract,"value":"0x0","gas":hex_quantity(self.policy.gas_limit),"data":envelope.call_data});
        let simulation = s.agree("eth_call", json!([call, hex_quantity(block.number)]))?;
        if simulation.as_str() != Some("0x") {
            return Err(Error::VerificationFailed);
        }
        s.recheck(&block)
    }
    fn verify_receipt(
        &self,
        evidence: &ReceiptEvidence,
        envelope: &PrivateEnvelope,
        verified: &VerifiedOperation,
    ) -> Result<()> {
        let mut actual = self
            .operation_receipt(
                &evidence.request_id,
                envelope,
                verified,
                &evidence.transaction_hash,
            )?
            .ok_or(Error::InvalidEvidence)?;
        if actual.finalized_height < evidence.finalized_height {
            return Err(Error::InvalidEvidence);
        }
        actual.finalized_height = evidence.finalized_height;
        if actual != *evidence {
            return Err(Error::InvalidEvidence);
        }
        Ok(())
    }
    fn verify_reorg(&self, evidence: &ReorgEvidence) -> Result<()> {
        if evidence.chain_id != CANDIDATE_CHAIN_ID {
            return Err(Error::InvalidEvidence);
        }
        let mut s = self.session();
        s.check_chain()?;
        let block = s.block(evidence.previous_block_number)?;
        if block.hash != evidence.canonical_block_hash || block.hash == evidence.previous_block_hash
        {
            return Err(Error::InvalidEvidence);
        }
        Ok(())
    }
}

impl Session<'_> {
    fn call(&mut self, endpoint: usize, method: &'static str, params: Value) -> Result<Value> {
        self.calls = self.calls.checked_add(1).ok_or(Error::Rpc)?;
        let remaining = SESSION_DEADLINE
            .checked_sub(self.started.elapsed())
            .ok_or(Error::Rpc)?;
        if self.calls > 192 {
            return Err(Error::Rpc);
        }
        let id = self.rpc.ids.fetch_add(1, Ordering::Relaxed);
        if id == u64::MAX {
            return Err(Error::Rpc);
        }
        let response = self
            .rpc
            .client
            .post(self.rpc.urls[endpoint].clone())
            .header("accept", "application/json")
            .json(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .timeout(remaining.min(Duration::from_secs(6)))
            .send()
            .map_err(|_| Error::Rpc)?;
        if response.status().as_u16() != 200
            || response
                .content_length()
                .is_some_and(|n| n > MAX_RPC_BYTES as u64)
        {
            return Err(Error::Rpc);
        }
        let mut body = Vec::new();
        response
            .take((MAX_RPC_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| Error::Rpc)?;
        if body.len() > MAX_RPC_BYTES {
            return Err(Error::Rpc);
        }
        let value = strict_json(&body)?;
        let object = value.as_object().ok_or(Error::Rpc)?;
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || object.get("id").and_then(Value::as_u64) != Some(id)
            || object.len() != 3
            || !object.contains_key("result")
            || object.contains_key("error")
        {
            return Err(Error::Rpc);
        }
        Ok(object["result"].clone())
    }
    fn agree(&mut self, method: &'static str, params: Value) -> Result<Value> {
        let a = self.call(0, method, params.clone())?;
        let b = self.call(1, method, params)?;
        if a != b {
            return Err(Error::InvalidEvidence);
        }
        Ok(a)
    }
    fn check_chain(&mut self) -> Result<()> {
        if quantity_u64(
            self.agree("eth_chainId", json!([]))?
                .as_str()
                .ok_or(Error::Rpc)?,
        )? != CANDIDATE_CHAIN_ID
        {
            return Err(Error::InvalidEvidence);
        }
        Ok(())
    }
    fn block(&mut self, number: u64) -> Result<BlockHeader> {
        let a = parse_header(&self.call(
            0,
            "eth_getBlockByNumber",
            json!([hex_quantity(number), false]),
        )?)?;
        let b = parse_header(&self.call(
            1,
            "eth_getBlockByNumber",
            json!([hex_quantity(number), false]),
        )?)?;
        if a != b || a.number != number {
            return Err(Error::InvalidEvidence);
        }
        Ok(a)
    }
    fn complete_block_logs(&mut self, number: u64) -> Result<(BlockHeader, Vec<PoolLog>)> {
        let a = self.call(
            0,
            "eth_getBlockByNumber",
            json!([hex_quantity(number), false]),
        )?;
        let b = self.call(
            1,
            "eth_getBlockByNumber",
            json!([hex_quantity(number), false]),
        )?;
        let block = parse_header(&a)?;
        let transactions = block_transactions(&a)?;
        if block.number != number
            || parse_header(&b)? != block
            || block_transactions(&b)? != transactions
        {
            return Err(Error::InvalidEvidence);
        }
        // eth_getLogs may omit an old-tree tail while still returning a valid
        // later tree. Obtain ALL receipts independently and validate exact
        // transaction coverage before selecting only the fixed pool's logs.
        let a = parse_block_receipts(
            &self.call(0, "eth_getBlockReceipts", json!([hex_quantity(number)]))?,
            &block,
            &transactions,
        )?;
        let b = parse_block_receipts(
            &self.call(1, "eth_getBlockReceipts", json!([hex_quantity(number)]))?,
            &block,
            &transactions,
        )?;
        if a != b {
            return Err(Error::InvalidEvidence);
        }
        let topics = pool_topics();
        let mut logs = Vec::new();
        for receipt in a {
            for log in receipt.logs {
                if log.address != self.rpc.policy.domain.pool_contract
                    || !log
                        .topics
                        .first()
                        .is_some_and(|topic| topics.contains(topic))
                {
                    continue;
                }
                if log.topics.len() != 1 || logs.len() >= 4096 {
                    return Err(Error::InvalidEvidence);
                }
                logs.push(log);
            }
        }
        self.recheck(&block)?;
        Ok((block, logs))
    }
    fn agreed_head(&mut self, tag: &str) -> Result<BlockHeader> {
        let a = parse_header(&self.call(0, "eth_getBlockByNumber", json!([tag, false]))?)?;
        let b = parse_header(&self.call(1, "eth_getBlockByNumber", json!([tag, false]))?)?;
        let number = a.number.min(b.number);
        let block = self.block(number)?;
        if a.number == number && a.hash != block.hash || b.number == number && b.hash != block.hash
        {
            return Err(Error::InvalidEvidence);
        }
        Ok(block)
    }
    fn recheck(&mut self, block: &BlockHeader) -> Result<()> {
        if self.block(block.number)? != *block {
            return Err(Error::InvalidEvidence);
        }
        Ok(())
    }
    fn require_fresh(&self, block: &BlockHeader) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Rpc)?
            .as_secs();
        if block.timestamp > now.saturating_add(15)
            || now.saturating_sub(block.timestamp) > self.rpc.config.max_head_age_seconds
        {
            return Err(Error::InvalidEvidence);
        }
        Ok(())
    }
    fn check_current_deployment(&mut self, block: &BlockHeader) -> Result<()> {
        let current_from = self
            .rpc
            .config
            .historical_deployments
            .last()
            .map_or(self.rpc.config.deployment_block, |epoch| epoch.until_block);
        if block.number < current_from {
            return Err(Error::InvalidEvidence);
        }
        self.check_deployment(block)
    }

    fn check_deployment(&mut self, block: &BlockHeader) -> Result<()> {
        if block.number < self.rpc.config.deployment_block {
            return Err(Error::InvalidEvidence);
        }
        let epoch = self
            .rpc
            .config
            .historical_deployments
            .iter()
            .find(|epoch| block.number < epoch.until_block);
        let token_not_yet_deployed = epoch.is_some_and(|epoch| epoch.token_not_yet_deployed);
        let (pool, token, implementation, storage) = epoch.map_or(
            (
                &self.rpc.config.pool_code_keccak256,
                &self.rpc.config.token_code_keccak256,
                &self.rpc.config.implementation_code_pins,
                &self.rpc.config.proxy_storage_pins,
            ),
            |epoch| {
                (
                    &epoch.pool_code_keccak256,
                    &epoch.token_code_keccak256,
                    &epoch.implementation_code_pins,
                    &epoch.proxy_storage_pins,
                )
            },
        );
        let mut pins = implementation.clone();
        pins.push(CodePin {
            address: self.rpc.policy.domain.pool_contract.clone(),
            keccak256: pool.clone(),
        });
        let token = token.clone();
        let storage = storage.clone();
        for pin in pins {
            let code = self.agree(
                "eth_getCode",
                json!([pin.address, hex_quantity(block.number)]),
            )?;
            let bytes = bounded_hex(code.as_str().ok_or(Error::Rpc)?, 131_072)?;
            if bytes.is_empty()
                || format!("0x{}", hex::encode(Keccak256::digest(bytes))) != pin.keccak256
            {
                return Err(Error::InvalidEvidence);
            }
        }
        // Pool and implementation code are always required above. Only an
        // explicit historical prefix may authenticate the absence of this
        // future token, rather than pretending it existed at pool creation.
        let code = self.agree(
            "eth_getCode",
            json!([
                self.rpc.policy.domain.token_contract,
                hex_quantity(block.number)
            ]),
        )?;
        let code = code.as_str().ok_or(Error::Rpc)?;
        let bytes = if token_not_yet_deployed && code == "0x" {
            Vec::new()
        } else {
            bounded_hex(code, 131_072)?
        };
        if bytes.is_empty() != token_not_yet_deployed
            || format!("0x{}", hex::encode(Keccak256::digest(bytes))) != token
        {
            return Err(Error::InvalidEvidence);
        }
        for pin in storage {
            if self
                .agree(
                    "eth_getStorageAt",
                    json!([pin.address, pin.slot, hex_quantity(block.number)]),
                )?
                .as_str()
                != Some(pin.value.as_str())
            {
                return Err(Error::InvalidEvidence);
            }
        }
        Ok(())
    }
    fn mapping(
        &mut self,
        function: &str,
        tree: u64,
        key: &str,
        block: &BlockHeader,
    ) -> Result<bool> {
        let data = view_call(function, &[word(tree.into()), bounded_word(key)?]);
        let value = self.agree("eth_call",json!([{"to":self.rpc.policy.domain.pool_contract,"data":data},hex_quantity(block.number)]))?;
        let result = bounded_word(value.as_str().ok_or(Error::Rpc)?)?;
        if result == word(0u8.into()) {
            Ok(false)
        } else if result == word(1u8.into()) {
            Ok(true)
        } else {
            Err(Error::InvalidEvidence)
        }
    }
    fn pool_getter(&mut self, function: &str, block: &BlockHeader) -> Result<[u8; 32]> {
        let data = view_call(function, &[]);
        let value=self.agree("eth_call",json!([{"to":self.rpc.policy.domain.pool_contract,"data":data},hex_quantity(block.number)]))?;
        bounded_word(value.as_str().ok_or(Error::Rpc)?)
    }
    fn check_private_state(
        &mut self,
        envelope: &PrivateEnvelope,
        verified: &VerifiedOperation,
        block: &BlockHeader,
    ) -> Result<()> {
        if verified.tree_number > 65535
            || !self.mapping(
                "rootHistory(uint256,bytes32)",
                verified.tree_number,
                &envelope.state_root,
                block,
            )?
        {
            return Err(Error::InvalidEvidence);
        }
        for nullifier in &envelope.nullifiers {
            if self.mapping(
                "nullifiers(uint256,bytes32)",
                verified.tree_number,
                nullifier,
                block,
            )? {
                return Err(Error::Conflict);
            }
        }
        let pin = self
            .rpc
            .config
            .verification_keys
            .iter()
            .find(|pin| {
                pin.inputs as usize == envelope.nullifiers.len()
                    && pin.outputs as usize == envelope.output_commitments.len()
            })
            .ok_or(Error::InvalidEvidence)?
            .clone();
        let data = view_call(
            "getVerificationKey(uint256,uint256)",
            &[
                word((pin.inputs as u64).into()),
                word((pin.outputs as u64).into()),
            ],
        );
        let key = self.agree("eth_call",json!([{"to":self.rpc.policy.domain.pool_contract,"data":data},hex_quantity(block.number)]))?;
        let bytes = bounded_hex(key.as_str().ok_or(Error::Rpc)?, 131_072)?;
        if format!("0x{}", hex::encode(Keccak256::digest(bytes))) != pin.encoded_key_keccak256 {
            return Err(Error::InvalidEvidence);
        }
        Ok(())
    }
    fn pool_receipt(&mut self, hash: &str, require_operator: bool) -> Result<Option<PoolReceipt>> {
        let value = self.agree("eth_getTransactionReceipt", json!([hash]))?;
        if value.is_null() {
            return Ok(None);
        }
        if string(&value, "transactionHash")? != hash
            || require_operator
                && lower_address(string(&value, "from")?)? != self.rpc.policy.broadcaster_address
            || lower_address(string(&value, "to")?)? != self.rpc.policy.domain.pool_contract
        {
            return Err(Error::InvalidEvidence);
        }
        let transaction = self.agree("eth_getTransactionByHash", json!([hash]))?;
        if string(&transaction, "hash")? != hash
            || quantity_u64(string(&transaction, "chainId")?)? != CANDIDATE_CHAIN_ID
            || require_operator && string(&transaction, "type")? != "0x2"
            || !quantity(string(&transaction, "value")?)?.is_zero()
            || require_operator
                && lower_address(string(&transaction, "from")?)?
                    != self.rpc.policy.broadcaster_address
            || lower_address(string(&transaction, "to")?)? != self.rpc.policy.domain.pool_contract
            || string(&transaction, "blockHash")? != string(&value, "blockHash")?
            || string(&transaction, "blockNumber")? != string(&value, "blockNumber")?
        {
            return Err(Error::InvalidEvidence);
        }
        // Client reconciliation accepts an exact pool proof broadcast by another
        // caller. The caller is not returned or persisted. Operator accounting
        // takes the stricter branch above and can never charge that foreign tx.
        if lower_address(string(&transaction, "from")?)? != lower_address(string(&value, "from")?)?
        {
            return Err(Error::InvalidEvidence);
        }
        quantity_u64(string(&transaction, "type")?)?;
        let input = string(&transaction, "input")?.to_string();
        bounded_hex(&input, 131_072)?;
        let status = string(&value, "status")?.to_string();
        if !["0x0", "0x1"].contains(&status.as_str()) {
            return Err(Error::InvalidEvidence);
        }
        let gas = quantity(string(&value, "gasUsed")?)?;
        if let Some(l1) = value.get("gasUsedForL1") {
            if quantity(l1.as_str().ok_or(Error::Rpc)?)? > gas {
                return Err(Error::InvalidEvidence);
            }
        }
        let block_number = string(&value, "blockNumber")?.to_string();
        quantity_u64(&block_number)?;
        let block_hash = string(&value, "blockHash")?.to_string();
        fixed_hex(&block_hash, 32)?;
        let mut logs = Vec::new();
        let mut seen_log_indices = BTreeSet::new();
        let mut transaction_index = None;
        for item in value
            .get("logs")
            .and_then(Value::as_array)
            .ok_or(Error::Rpc)?
        {
            if logs.len() >= 64 {
                return Err(Error::InvalidEvidence);
            }
            let log = parse_log(item)?;
            if log.transaction_hash != hash
                || log.block_number != block_number
                || log.block_hash != block_hash
                || log.removed
                || log.address != self.rpc.policy.domain.pool_contract
                || !seen_log_indices.insert(quantity_u64(&log.log_index)?)
                || transaction_index
                    .as_ref()
                    .is_some_and(|index| index != &log.transaction_index)
            {
                return Err(Error::InvalidEvidence);
            }
            transaction_index = Some(log.transaction_index.clone());
            logs.push(log);
        }
        Ok(Some(PoolReceipt {
            transaction_hash: hash.into(),
            block_hash,
            block_number,
            status,
            gas_used: string(&value, "gasUsed")?.into(),
            effective_gas_price: string(&value, "effectiveGasPrice")?.into(),
            logs,
            transaction_input: input,
        }))
    }
}

fn validate_deployment_pins(
    pool: &str,
    token: &str,
    implementation: &[CodePin],
    storage: &[StoragePin],
) -> Result<()> {
    if implementation.is_empty()
        || implementation.len() > 8
        || storage.is_empty()
        || storage.len() > 8
    {
        return Err(Error::InvalidPolicy);
    }
    fixed_hex(pool, 32)?;
    fixed_hex(token, 32)?;
    let mut addresses = BTreeSet::new();
    for pin in implementation {
        fixed_hex(&pin.address, 20)?;
        fixed_hex(&pin.keccak256, 32)?;
        if !addresses.insert(&pin.address) {
            return Err(Error::InvalidPolicy);
        }
    }
    let mut slots = BTreeSet::new();
    for pin in storage {
        fixed_hex(&pin.address, 20)?;
        fixed_hex(&pin.slot, 32)?;
        bounded_word(&pin.value)?;
        if !slots.insert((&pin.address, &pin.slot)) {
            return Err(Error::InvalidPolicy);
        }
    }
    Ok(())
}

fn parse_header(value: &Value) -> Result<BlockHeader> {
    let hash = string(value, "hash")?.to_string();
    let parent_hash = string(value, "parentHash")?.to_string();
    fixed_hex(&hash, 32)?;
    fixed_hex(&parent_hash, 32)?;
    Ok(BlockHeader {
        number: quantity_u64(string(value, "number")?)?,
        hash,
        parent_hash,
        timestamp: quantity_u64(string(value, "timestamp")?)?,
    })
}

fn block_transactions(value: &Value) -> Result<Vec<String>> {
    let array = value
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or(Error::Rpc)?;
    if array.len() > MAX_BLOCK_TRANSACTIONS {
        return Err(Error::InvalidEvidence);
    }
    let mut transactions = Vec::with_capacity(array.len());
    let mut seen = BTreeSet::new();
    for value in array {
        let hash = value.as_str().ok_or(Error::InvalidEvidence)?;
        fixed_hex(hash, 32)?;
        if !seen.insert(hash) {
            return Err(Error::InvalidEvidence);
        }
        transactions.push(hash.to_owned());
    }
    Ok(transactions)
}

fn parse_block_receipts(
    value: &Value,
    block: &BlockHeader,
    transactions: &[String],
) -> Result<Vec<BlockReceipt>> {
    let array = value.as_array().ok_or(Error::Rpc)?;
    if array.len() != transactions.len() || array.len() > MAX_BLOCK_TRANSACTIONS {
        return Err(Error::InvalidEvidence);
    }
    let mut receipts = Vec::with_capacity(array.len());
    let mut next_log_index = 0u64;
    for (index, (value, expected_hash)) in array.iter().zip(transactions).enumerate() {
        if string(value, "transactionHash")? != expected_hash
            || quantity_u64(string(value, "transactionIndex")?)? != index as u64
            || string(value, "blockHash")? != block.hash
            || quantity_u64(string(value, "blockNumber")?)? != block.number
        {
            return Err(Error::InvalidEvidence);
        }
        let status = quantity_u64(string(value, "status")?)?;
        let gas_used = quantity(string(value, "gasUsed")?)?;
        let effective_gas_price = quantity(string(value, "effectiveGasPrice")?)?;
        let raw_logs = value
            .get("logs")
            .and_then(Value::as_array)
            .ok_or(Error::Rpc)?;
        if status > 1
            || status == 0 && !raw_logs.is_empty()
            || raw_logs.len() > MAX_BLOCK_LOGS.saturating_sub(next_log_index as usize)
        {
            return Err(Error::InvalidEvidence);
        }
        let mut logs = Vec::with_capacity(raw_logs.len());
        for value in raw_logs {
            // Other contracts may legitimately emit LOG0. They still count
            // toward complete global log-index coverage before being filtered.
            let log = parse_raw_log(value, true)?;
            if log.transaction_hash != *expected_hash
                || quantity_u64(&log.transaction_index)? != index as u64
                || log.block_hash != block.hash
                || quantity_u64(&log.block_number)? != block.number
                || log.removed
                || quantity_u64(&log.log_index)? != next_log_index
            {
                return Err(Error::InvalidEvidence);
            }
            next_log_index += 1;
            logs.push(log);
        }
        receipts.push(BlockReceipt {
            transaction_hash: expected_hash.clone(),
            transaction_index: index as u64,
            status,
            gas_used,
            effective_gas_price,
            logs,
        });
    }
    Ok(receipts)
}

fn parse_log(value: &Value) -> Result<PoolLog> {
    parse_raw_log(value, false)
}

fn parse_raw_log(value: &Value, allow_empty_topics: bool) -> Result<PoolLog> {
    let topics: Vec<String> = value
        .get("topics")
        .and_then(Value::as_array)
        .ok_or(Error::Rpc)?
        .iter()
        .map(|value| {
            let value = value.as_str().ok_or(Error::Rpc)?;
            fixed_hex(value, 32)?;
            Ok(value.to_string())
        })
        .collect::<Result<_>>()?;
    if (!allow_empty_topics && topics.is_empty()) || topics.len() > 4 {
        return Err(Error::InvalidEvidence);
    }
    let data = string(value, "data")?.to_string();
    if data != "0x" {
        bounded_hex(&data, 131_072)?;
    }
    let block_hash = string(value, "blockHash")?.to_string();
    let transaction_hash = string(value, "transactionHash")?.to_string();
    fixed_hex(&block_hash, 32)?;
    fixed_hex(&transaction_hash, 32)?;
    let block_number = string(value, "blockNumber")?.to_string();
    let transaction_index = string(value, "transactionIndex")?.to_string();
    let log_index = string(value, "logIndex")?.to_string();
    quantity_u64(&block_number)?;
    quantity_u64(&transaction_index)?;
    quantity_u64(&log_index)?;
    Ok(PoolLog {
        address: lower_address(string(value, "address")?)?,
        topics,
        data,
        block_hash,
        block_number,
        transaction_hash,
        transaction_index,
        log_index,
        removed: value
            .get("removed")
            .and_then(Value::as_bool)
            .ok_or(Error::Rpc)?,
    })
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or(Error::Rpc)
}
fn lower_address(value: &str) -> Result<String> {
    let result = value.to_ascii_lowercase();
    fixed_hex(&result, 20)?;
    Ok(result)
}
fn hex_quantity(value: u64) -> String {
    format!("0x{value:x}")
}
fn quantity(value: &str) -> Result<U256> {
    if !value.starts_with("0x")
        || value.len() < 3
        || value.len() > 66
        || value.len() > 3 && value.as_bytes()[2] == b'0'
        || !value[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidEvidence);
    }
    U256::from_str_radix(&value[2..], 16).map_err(|_| Error::InvalidEvidence)
}
fn quantity_u64(value: &str) -> Result<u64> {
    let number = quantity(value)?;
    if number > u64::MAX.into() {
        return Err(Error::InvalidEvidence);
    }
    Ok(number.low_u64())
}
fn word(value: U256) -> [u8; 32] {
    value.to_big_endian()
}
fn bounded_word(value: &str) -> Result<[u8; 32]> {
    bounded_hex(value, 32)?
        .try_into()
        .map_err(|_| Error::InvalidEncoding)
}
fn view_call(function: &str, values: &[[u8; 32]]) -> String {
    let selector = Keccak256::digest(function.as_bytes());
    let mut bytes = selector[..4].to_vec();
    for value in values {
        bytes.extend_from_slice(value);
    }
    format!("0x{}", hex::encode(bytes))
}
fn topic(signature: &str) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(signature.as_bytes())))
}

fn pool_topics() -> Vec<String> {
    [
        "Nullified(uint16,bytes32[])",
        "Transact(uint256,uint256,bytes32[],(bytes32[4],bytes32,bytes32,bytes,bytes)[])",
        "Shield(uint256,uint256,(bytes32,(uint8,address,uint256),uint120)[],(bytes32[3],bytes32)[],uint256[])",
        "Shield(uint256,uint256,(bytes32,(uint8,address,uint256),uint120)[],(bytes32[3],bytes32)[])",
        "GeneratedCommitmentBatch(uint256,uint256,(uint256,(uint8,address,uint256),uint120)[],uint256[2][])",
        "CommitmentBatch(uint256,uint256,uint256[],(uint256[4],uint256[2],uint256[])[])",
        "Nullifiers(uint256,uint256[])",
    ].iter().map(|signature| topic(signature)).collect()
}

fn match_private_logs(
    logs: &[PoolLog],
    envelope: &PrivateEnvelope,
    verified: &VerifiedOperation,
) -> Result<()> {
    if logs.len() != 2 {
        return Err(Error::InvalidEvidence);
    }
    let nullified = topic("Nullified(uint16,bytes32[])");
    let transact =
        topic("Transact(uint256,uint256,bytes32[],(bytes32[4],bytes32,bytes32,bytes,bytes)[])");
    let nullifier_log = logs
        .iter()
        .find(|log| log.topics == [nullified.clone()])
        .ok_or(Error::InvalidEvidence)?;
    let output_log = logs
        .iter()
        .find(|log| log.topics == [transact.clone()])
        .ok_or(Error::InvalidEvidence)?;
    let mut expected = Vec::new();
    expected.extend(word(verified.tree_number.into()));
    expected.extend(word(64u64.into()));
    expected.extend(word(envelope.nullifiers.len().into()));
    for nullifier in &envelope.nullifiers {
        expected.extend(bounded_word(nullifier)?);
    }
    if nullifier_log.data != format!("0x{}", hex::encode(expected)) {
        return Err(Error::InvalidEvidence);
    }
    let body = bounded_hex(&output_log.data, 131_072)?;
    if body.len() < 128 {
        return Err(Error::InvalidEvidence);
    }
    let output_tree = U256::from_big_endian(&body[..32]);
    let start = U256::from_big_endian(&body[32..64]);
    if output_tree > 65535u64.into()
        || start
            .checked_add(U256::from(envelope.output_commitments.len()))
            .ok_or(Error::InvalidEvidence)?
            > 65536u64.into()
    {
        return Err(Error::InvalidEvidence);
    }
    let count = envelope.output_commitments.len();
    let mut expected = body[..64].to_vec();
    expected.extend(word(128u64.into()));
    expected.extend(word((160 + count * 32).into()));
    expected.extend(word(count.into()));
    for commitment in &envelope.output_commitments {
        expected.extend(bounded_word(commitment)?);
    }
    expected.extend(word(count.into()));
    let mut tuples = Vec::new();
    let mut offset = count * 32;
    for encoded in &envelope.encrypted_outputs {
        let tuple = bounded_hex(encoded, 8192)?;
        if tuple.len() < 32 || tuple[..32] != word(32u64.into()) {
            return Err(Error::InvalidEvidence);
        }
        expected.extend(word(offset.into()));
        offset = offset
            .checked_add(tuple.len() - 32)
            .ok_or(Error::Arithmetic)?;
        tuples.extend_from_slice(&tuple[32..]);
    }
    expected.extend(tuples);
    if expected != body {
        return Err(Error::InvalidEvidence);
    }
    Ok(())
}

/// Recursively reject duplicate JSON keys before RPC result normalization.
fn strict_json(bytes: &[u8]) -> Result<Value> {
    struct Strict(Value);
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Strict;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("unambiguous JSON")
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    v: bool,
                ) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Bool(v)))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::from(v)))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::from(v)))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::String(v.into())))
                }
                fn visit_string<E: serde::de::Error>(
                    self,
                    v: String,
                ) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::String(v)))
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Strict(value)) = seq.next_element()? {
                        values.push(value);
                    }
                    Ok(Strict(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some((key, Strict(value))) = map.next_entry::<String, Strict>()? {
                        if values.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate JSON field"));
                        }
                    }
                    Ok(Strict(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Strict>(bytes)
        .map(|value| value.0)
        .map_err(|_| Error::Rpc)
}
