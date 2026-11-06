//! Actual HTTP/RLP/OHTTP integration against two loopback RPC fixtures. The ABI
//! vectors were generated independently with the SDK's ethers Interface. Their
//! proof is deliberately INVALID; only this cfg(test) adapter admits it.
use super::*;
use crate::rpc::{
    ArbitrumRpc, CodePin, HistoricalDeployment, PoolRequest, RpcConfig, StoragePin,
    VerificationKeyPin,
};
use crate::signer::NativeTreasurySigner;
use crate::tests::{hash, policy};
use crate::worker::OperatorWorker;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use std::{
    collections::BTreeMap,
    convert::Infallible,
    net::SocketAddr,
    sync::mpsc,
    time::{SystemTime, UNIX_EPOCH},
};
use wallet_core::private_payment::{envelope_digest, PrivateEnvelope};
use zeroize::Zeroizing;

fn keccak(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(bytes)))
}
fn block_hash(number: u64) -> String {
    keccak(format!("public-offline-block-{number}").as_bytes())
}
fn quantity(number: u64) -> String {
    format!("0x{number:x}")
}
fn abi() -> Value {
    serde_json::from_str(include_str!("../fixtures/abi-case.json")).unwrap()
}

pub(crate) fn abi_envelope() -> (AdmissionPolicy, PrivateEnvelope, VerifiedOperation) {
    let fixture = abi();
    let mut p = policy();
    p.allowed_selectors = vec![fixture["selector"].as_str().unwrap().into()];
    let mut env = crate::tests::envelope(11);
    env.tree_number = 1;
    env.state_root = fixture["state_root"].as_str().unwrap().into();
    env.nullifiers = serde_json::from_value(fixture["nullifiers"].clone()).unwrap();
    env.output_commitments = serde_json::from_value(fixture["output_commitments"].clone()).unwrap();
    env.encrypted_outputs = serde_json::from_value(fixture["encrypted_outputs"].clone()).unwrap();
    env.proof = fixture["proof"].as_str().unwrap().into();
    env.call_data = fixture["call_data"].as_str().unwrap().into();
    env.fee.fee_output_commitment = env.output_commitments.last().unwrap().clone();
    env.operation_digest = envelope_digest(&env).unwrap();
    let mut verified = crate::tests::verified(&env);
    verified.additional_native_fee_cap_wei = "0".into();
    (p, env, verified)
}

struct InvalidProofFixture;
impl ProofVerifier for InvalidProofFixture {
    fn verify(&self, env: &PrivateEnvelope) -> Result<VerifiedOperation> {
        let mut result = crate::tests::verified(env);
        result.additional_native_fee_cap_wei = "0".into();
        Ok(result)
    }
}

struct RpcState {
    head: u64,
    timestamp: u64,
    balance: u64,
    next_leaf_index: u64,
    reorg: bool,
    mined_block: u64,
    external_sender: bool,
    records: BTreeMap<String, (Value, Value)>,
    block_transactions: BTreeMap<u64, Vec<String>>,
    block_receipts: BTreeMap<u64, Value>,
    block_receipts_secondary: Option<Value>,
    reorg_after_block_receipts: bool,
    policy: AdmissionPolicy,
    envelope: PrivateEnvelope,
    sent: Vec<String>,
    mined: bool,
    reverted: bool,
    wrong_ack: bool,
    wrong_chain: bool,
    wrong_code: bool,
    unknown_root: bool,
    spent: bool,
    tamper_output: bool,
    duplicate_log: bool,
    wrong_input: bool,
    duplicate_json: bool,
    oversized: bool,
    historical_code: bool,
    token_deployment_block: Option<u64>,
    token_absence_disagreement: bool,
}
impl RpcState {
    fn header(&self, number: u64) -> Value {
        json!({"number":quantity(number),"hash":if self.reorg&&number==122{hash(88)}else{block_hash(number)},"parentHash":block_hash(number.saturating_sub(1)),"timestamp":quantity(self.timestamp),"baseFeePerGas":"0x1","transactions":self.block_transactions.get(&number).cloned().unwrap_or_default()})
    }
    fn transaction_hash(&self) -> Option<String> {
        self.sent
            .last()
            .map(|raw| keccak(&hex::decode(&raw[2..]).unwrap()))
    }
    fn logs(&self) -> Vec<Value> {
        if !self.mined || self.reverted {
            return Vec::new();
        }
        let transaction_hash = self.transaction_hash().unwrap();
        abi()["logs"].as_array().unwrap().iter().enumerate().map(|(index,log)|{
            let mut data=log["data"].as_str().unwrap().to_string();
            if self.tamper_output && index==1 {let last=data.pop().unwrap();data.push(if last=='0' {'1'}else{'0'});}
            json!({"address":self.policy.domain.pool_contract,"topics":log["topics"],"data":data,
                "blockHash":block_hash(self.mined_block),"blockNumber":quantity(self.mined_block),"transactionHash":transaction_hash,
                "transactionIndex":"0x0","logIndex":quantity(if self.duplicate_log {0}else{index as u64}),"removed":false})
        }).collect()
    }
    fn result(&mut self, method: &str, params: &Value, endpoint: usize) -> Value {
        match method {
            "eth_chainId" => json!(if self.wrong_chain && endpoint == 1 {
                "0x1"
            } else {
                "0xa4b1"
            }),
            "eth_getBlockByNumber" => {
                let requested = params[0].as_str().unwrap();
                let number = if ["latest", "finalized"].contains(&requested) {
                    self.head
                } else {
                    u64::from_str_radix(&requested[2..], 16).unwrap()
                };
                self.header(number)
            }
            "eth_getCode" => {
                if self.wrong_code {
                    return json!("0x6001");
                }
                let number = u64::from_str_radix(&params[1].as_str().unwrap()[2..], 16).unwrap();
                let address = params[0].as_str().unwrap();
                if address == self.policy.domain.token_contract
                    && self
                        .token_deployment_block
                        .is_some_and(|boundary| number < boundary)
                {
                    return json!(if self.token_absence_disagreement && endpoint == 1 {
                        "0x60006000"
                    } else {
                        "0x"
                    });
                }
                if !self.historical_code {
                    return json!("0x60006000");
                }
                let code = if address == format!("0x{}", hex::encode([45; 20])) {
                    if number >= 118 {
                        "0x60006000"
                    } else {
                        "0x"
                    }
                } else if address == format!("0x{}", hex::encode([46; 20])) {
                    if number >= 110 {
                        "0x60016001"
                    } else {
                        "0x"
                    }
                } else if address == format!("0x{}", hex::encode([47; 20])) {
                    if number >= 100 {
                        "0x60026002"
                    } else {
                        "0x"
                    }
                } else if address == self.policy.domain.pool_contract
                    || address == self.policy.domain.token_contract
                {
                    if number >= 118 {
                        "0x60006000"
                    } else if number >= 110 {
                        "0x60016001"
                    } else {
                        "0x60026002"
                    }
                } else {
                    "0x"
                };
                json!(code)
            }
            "eth_getStorageAt" => {
                let number = u64::from_str_radix(&params[2].as_str().unwrap()[2..], 16).unwrap();
                json!(hash(if !self.historical_code || number >= 118 {
                    90
                } else if number >= 110 {
                    91
                } else {
                    92
                }))
            }
            "eth_getBalance" => json!(quantity(self.balance)),
            "eth_getTransactionCount" => json!("0x0"),
            "eth_maxPriorityFeePerGas" => json!("0x1"),
            "eth_estimateGas" => json!("0x10000"),
            "eth_call" => {
                let call = &params[0];
                let data = call["data"].as_str().unwrap();
                let fixture = abi();
                if call.get("from").is_some() {
                    assert_eq!(data, self.envelope.call_data);
                    assert_eq!(call["value"], "0x0");
                    return json!("0x");
                }
                let selector = &data[..10];
                if selector == fixture["view_selectors"]["rootHistory"] {
                    json!(format!("0x{:064x}", u8::from(!self.unknown_root)))
                } else if selector == fixture["view_selectors"]["nullifiers"] {
                    json!(format!("0x{:064x}", u8::from(self.spent)))
                } else if selector == fixture["view_selectors"]["getVerificationKey"] {
                    json!("0x0102")
                } else if selector == fixture["view_selectors"]["treeNumber"] {
                    json!(format!("0x{:064x}", 1))
                } else if selector == fixture["view_selectors"]["nextLeafIndex"] {
                    json!(format!("0x{:064x}", self.next_leaf_index))
                } else if selector == fixture["view_selectors"]["merkleRoot"] {
                    json!(self.envelope.state_root)
                } else {
                    panic!("unexpected fixture view selector")
                }
            }
            "eth_sendRawTransaction" => {
                let raw = params[0].as_str().unwrap().to_string();
                let digest = keccak(&hex::decode(&raw[2..]).unwrap());
                self.sent.push(raw);
                self.mined = false;
                json!(if self.wrong_ack { hash(99) } else { digest })
            }
            "eth_getTransactionReceipt" => {
                if !self.mined || self.transaction_hash().as_deref() != params[0].as_str() {
                    return self
                        .records
                        .get(params[0].as_str().unwrap())
                        .map(|record| record.0.clone())
                        .unwrap_or(Value::Null);
                }
                json!({"transactionHash":self.transaction_hash().unwrap(),"blockHash":block_hash(self.mined_block),"blockNumber":quantity(self.mined_block),
                    "from":if self.external_sender {"0x2222222222222222222222222222222222222222"}else{&self.policy.broadcaster_address},"to":self.policy.domain.pool_contract,
                    "status":if self.reverted {"0x0"}else{"0x1"},"gasUsed":"0xc350","gasUsedForL1":"0x200","effectiveGasPrice":"0x2","logs":self.logs()})
            }
            "eth_getBlockReceipts" => {
                let number = u64::from_str_radix(&params[0].as_str().unwrap()[2..], 16).unwrap();
                let response = if endpoint == 1 {
                    self.block_receipts_secondary.clone()
                } else {
                    None
                }
                .unwrap_or_else(|| {
                    self.block_receipts
                        .get(&number)
                        .cloned()
                        .unwrap_or(Value::Null)
                });
                if self.reorg_after_block_receipts && endpoint == 1 {
                    self.reorg = true;
                }
                response
            }
            "eth_getTransactionByHash" => {
                if !self.mined || self.transaction_hash().as_deref() != params[0].as_str() {
                    return self
                        .records
                        .get(params[0].as_str().unwrap())
                        .map(|record| record.1.clone())
                        .unwrap_or(Value::Null);
                }
                json!({"hash":self.transaction_hash().unwrap(),"chainId":"0xa4b1","type":if self.external_sender {"0x0"}else{"0x2"},"value":"0x0",
                    "from":if self.external_sender {"0x2222222222222222222222222222222222222222"}else{&self.policy.broadcaster_address},"to":self.policy.domain.pool_contract,"blockHash":block_hash(self.mined_block),"blockNumber":quantity(self.mined_block),
                    "input":if self.wrong_input {"0xdeadbeef"}else{&self.envelope.call_data}})
            }
            "eth_getLogs" => {
                // No wallet address or per-user filter is permitted on this port.
                assert_eq!(params[0]["address"], self.policy.domain.pool_contract);
                assert_eq!(params[0].as_object().unwrap().len(), 4);
                assert_eq!(params[0]["topics"].as_array().unwrap().len(), 1);
                json!(self.logs())
            }
            _ => panic!("unexpected fixture RPC method {method}"),
        }
    }
}

struct RunningRpc {
    address: SocketAddr,
    stop: tokio::sync::watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for RunningRpc {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn start_rpc(state: Arc<Mutex<RpcState>>, endpoint: usize) -> RunningRpc {
    let (send, receive) = mpsc::channel();
    let (stop, mut shutdown) = tokio::sync::watch::channel(false);
    let thread = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();send.send(listener.local_addr().unwrap()).unwrap();
            let mut connections=tokio::task::JoinSet::new();
            loop {tokio::select!{
                _=shutdown.changed()=>break,
                accepted=listener.accept()=>{
                    let(stream,_)=accepted.unwrap();let state=state.clone();
                    connections.spawn(async move {
                        let service=service_fn(move|request:Request<Incoming>|{
                            let state=state.clone();async move {
                                let body=request.into_body().collect().await.unwrap().to_bytes();assert!(body.len()<300000);
                                let value:Value=serde_json::from_slice(&body).unwrap();
                                let mut state=state.lock().unwrap();
                                let result=state.result(value["method"].as_str().unwrap(),&value["params"],endpoint);
                                let output=if state.oversized {"x".repeat(1_048_577)}
                                    else if state.duplicate_json {format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":null,\"result\":{result}}}",value["id"])}
                                    else {json!({"jsonrpc":"2.0","id":value["id"],"result":result}).to_string()};
                                Ok::<_,Infallible>(Response::new(Full::new(Bytes::from(output))))
                            }
                        });
                        let _=hyper::server::conn::http1::Builder::new().keep_alive(false).serve_connection(TokioIo::new(stream),service).await;
                    });
                },
                Some(done)=connections.join_next()=>{done.unwrap();}
            }}
            while let Some(result)=connections.join_next().await {result.unwrap();}
        });
    });
    RunningRpc {
        address: receive.recv().unwrap(),
        stop,
        thread: Some(thread),
    }
}

struct Fixture {
    _servers: [RunningRpc; 2],
    _directory: tempfile::TempDir,
    state: Arc<Mutex<RpcState>>,
    rpc: Arc<ArbitrumRpc>,
    service: Arc<GasService>,
    worker: OperatorWorker,
    env: PrivateEnvelope,
    verified: VerifiedOperation,
    policy: AdmissionPolicy,
    rpc_config: RpcConfig,
}
fn fixture() -> Fixture {
    fixture_with_history(vec![])
}
fn fixture_with_history(history: Vec<HistoricalDeployment>) -> Fixture {
    let (p, env, verified) = abi_envelope();
    let state = Arc::new(Mutex::new(RpcState {
        head: 120,
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        balance: 10000000,
        next_leaf_index: 2,
        reorg: false,
        mined_block: 122,
        external_sender: false,
        records: BTreeMap::new(),
        block_transactions: BTreeMap::new(),
        block_receipts: BTreeMap::new(),
        block_receipts_secondary: None,
        reorg_after_block_receipts: false,
        policy: p.clone(),
        envelope: env.clone(),
        sent: vec![],
        mined: false,
        reverted: false,
        wrong_ack: false,
        wrong_chain: false,
        wrong_code: false,
        unknown_root: false,
        spent: false,
        tamper_output: false,
        duplicate_log: false,
        wrong_input: false,
        duplicate_json: false,
        oversized: false,
        historical_code: !history.is_empty(),
        token_deployment_block: None,
        token_absence_disagreement: false,
    }));
    let servers = [start_rpc(state.clone(), 0), start_rpc(state.clone(), 1)];
    let config = RpcConfig {
        primary_url: format!("http://{}", servers[0].address),
        secondary_url: format!("http://{}", servers[1].address),
        pool_code_keccak256: keccak(&[0x60, 0, 0x60, 0]),
        token_code_keccak256: keccak(&[0x60, 0, 0x60, 0]),
        implementation_code_pins: vec![CodePin {
            address: format!("0x{}", hex::encode([45; 20])),
            keccak256: keccak(&[0x60, 0, 0x60, 0]),
        }],
        proxy_storage_pins: vec![StoragePin {
            address: p.domain.pool_contract.clone(),
            slot: hash(91),
            value: hash(90),
        }],
        historical_deployments: history,
        verification_keys: vec![VerificationKeyPin {
            inputs: 2,
            outputs: 2,
            encoded_key_keccak256: keccak(&[1, 2]),
        }],
        max_head_age_seconds: 300,
        deployment_block: 100,
    };
    let rpc = Arc::new(ArbitrumRpc::local_fixture(config.clone(), p.clone()).unwrap());
    let directory = tempfile::tempdir().unwrap();
    let service = Arc::new(
        GasService::open_harness(
            &directory.path().join("journal"),
            p.clone(),
            Arc::new(InvalidProofFixture),
            rpc.clone(),
        )
        .unwrap(),
    );
    let signer = NativeTreasurySigner::from_key(Zeroizing::new([7; 32])).unwrap();
    let worker = OperatorWorker::new(service.clone(), rpc.clone(), signer).unwrap();
    worker.initialize_treasury().unwrap();
    Fixture {
        _servers: servers,
        _directory: directory,
        state,
        rpc,
        service,
        worker,
        env,
        verified,
        policy: p,
        rpc_config: config,
    }
}
fn mine(f: &Fixture, reverted: bool) {
    let mut state = f.state.lock().unwrap();
    state.mined_block = 122 + 4 * (state.sent.len() as u64 - 1);
    state.head = state.mined_block + 2;
    state.mined = true;
    state.reverted = reverted;
    let hash = state.transaction_hash().unwrap();
    let receipt = state.result("eth_getTransactionReceipt", &json!([hash]), 0);
    let tx = state.result("eth_getTransactionByHash", &json!([hash]), 0);
    state.records.insert(hash, (receipt, tx));
}

#[test]
fn actual_http_rpc_worker_signs_persists_reconciles_and_never_double_counts_l1_fee() {
    let f = fixture();
    let id = &f.env.operation_digest;
    f.service.admit(id, &encode(&f.env).unwrap(), 1500).unwrap();
    let pending = f.worker.process_once(id).unwrap();
    assert_eq!(pending.phase, JobPhase::Pending);
    assert_eq!(pending.broadcast_attempts, 1);
    let sent = f.state.lock().unwrap().sent.clone();
    assert_eq!(sent.len(), 1);
    let parsed = crate::transaction::inspect_outer(&sent[0], &f.policy, &f.verified).unwrap();
    assert_eq!(
        pending.transaction_hash.as_deref(),
        Some(parsed.transaction_hash.as_str())
    );
    assert_eq!(f.service.summary().unwrap().reserved_native_wei, "1000000");
    // Immediate reprocessing reconciles first and respects durable retry delay.
    assert_eq!(f.worker.process_once(id).unwrap().phase, JobPhase::Pending);
    assert_eq!(f.state.lock().unwrap().sent.len(), 1);
    mine(&f, false);
    let finalized = f.worker.process_once(id).unwrap();
    assert_eq!(finalized.phase, JobPhase::Finalized);
    let summary = f.service.summary().unwrap();
    assert_eq!(summary.reserved_native_wei, "0");
    assert_eq!(summary.finalized_native_charge_wei, "100000");
    assert_eq!(summary.earned_fee_units, "20000");
    assert_eq!(f.worker.process_once(id).unwrap(), finalized);
    assert_eq!(f.state.lock().unwrap().sent.len(), 1);
    let request = PoolRequest::Receipt {
        schema_version: 1,
        chain_id: 42161,
        transaction_hash: parsed.transaction_hash,
    };
    let response = f.rpc.pool_read(&request).unwrap();
    assert_eq!(response["transaction_input"], f.env.call_data);
    assert!(response["receipt"].get("transactionInput").is_none());
    assert_eq!(response.as_object().unwrap().len(), 6);
}

#[test]
fn uncertain_ack_keeps_identical_signed_bytes_and_revert_earns_no_provider_fee() {
    let f = fixture();
    f.state.lock().unwrap().wrong_ack = true;
    let id = &f.env.operation_digest;
    f.service.admit(id, &encode(&f.env).unwrap(), 1500).unwrap();
    let unknown = f.worker.process_once(id).unwrap();
    assert_eq!(unknown.phase, JobPhase::Unknown);
    assert_eq!(unknown.broadcast_attempts, 1);
    assert_eq!(f.service.summary().unwrap().reserved_native_wei, "1000000");
    assert_eq!(f.worker.process_once(id).unwrap(), unknown);
    assert_eq!(f.state.lock().unwrap().sent.len(), 1);
    mine(&f, true);
    assert_eq!(
        f.worker.process_once(id).unwrap().phase,
        JobPhase::FinalizedRevert
    );
    let summary = f.service.summary().unwrap();
    assert_eq!(summary.earned_fee_units, "0");
    assert_eq!(summary.finalized_native_charge_wei, "100000");
    assert_eq!(summary.reserved_native_wei, "0");
}

#[test]
fn independent_rpc_and_deployment_checks_reject_ambiguous_or_changed_state() {
    let f = fixture();
    for change in [0, 1, 2, 3, 4, 5] {
        {
            let mut state = f.state.lock().unwrap();
            state.wrong_chain = change == 0;
            state.wrong_code = change == 1;
            state.unknown_root = change == 2;
            state.spent = change == 3;
            state.duplicate_json = change == 4;
            state.oversized = change == 5;
        }
        assert!(f.rpc.verify_admission(&f.env, &f.verified).is_err());
    }
    assert!(f.state.lock().unwrap().sent.is_empty());
    assert_eq!(f.service.summary().unwrap().jobs, 0);
}

#[test]
fn receipt_needs_exact_ciphertexts_log_identity_and_canonical_outer_calldata() {
    let f = fixture();
    let id = &f.env.operation_digest;
    f.service.admit(id, &encode(&f.env).unwrap(), 1500).unwrap();
    f.worker.process_once(id).unwrap();
    mine(&f, false);
    let tx = f.state.lock().unwrap().transaction_hash().unwrap();
    for change in [0, 1, 2] {
        {
            let mut state = f.state.lock().unwrap();
            state.tamper_output = change == 0;
            state.duplicate_log = change == 1;
            state.wrong_input = change == 2;
        }
        assert!(f
            .rpc
            .operation_receipt(id, &f.env, &f.verified, &tx)
            .is_err());
        assert_eq!(f.service.summary().unwrap().reserved_native_wei, "1000000");
        assert_eq!(f.service.summary().unwrap().earned_fee_units, "0");
    }
    f.state.lock().unwrap().wrong_input = false;
    assert!(f
        .rpc
        .operation_receipt(id, &f.env, &f.verified, &tx)
        .unwrap()
        .is_some());
}

#[test]
fn pool_port_is_fixed_bulk_data_and_padded_nullifiers_not_arbitrary_rpc() {
    let f = fixture();
    for kind in ["head", "tip"] {
        let request: PoolRequest =
            serde_json::from_value(json!({"schema_version":1,"chain_id":42161,"kind":kind}))
                .unwrap();
        let response = f.rpc.pool_read(&request).unwrap();
        assert_eq!(response["kind"], kind);
        assert_eq!(response["block"]["number"], 120);
    }
    let root = PoolRequest::Root {
        schema_version: 1,
        chain_id: 42161,
        tree_number: 1,
        root: f.env.state_root.clone(),
    };
    assert_eq!(f.rpc.pool_read(&root).unwrap()["known"], true);
    let nullifiers = PoolRequest::Nullifiers {
        schema_version: 1,
        chain_id: 42161,
        tree_number: 1,
        nullifiers: (1..=16).map(hash).collect(),
    };
    assert_eq!(
        f.rpc.pool_read(&nullifiers).unwrap()["spent"],
        json!(vec![false; 16])
    );
    let bad:PoolRequest=serde_json::from_value(json!({"schema_version":1,"chain_id":42161,"kind":"nullifiers","tree_number":1,"nullifiers":[hash(1)]})).unwrap();
    assert!(bad.validate().is_err());
    assert!(serde_json::from_value::<PoolRequest>(json!({"schema_version":1,"chain_id":42161,"kind":"logs","from_block":100,"to_block":120,"recipient":hash(1)})).is_err());
    let wrong_window = PoolRequest::Logs {
        schema_version: 1,
        chain_id: 42161,
        from_block: 103,
        to_block: 104,
    };
    assert!(f.rpc.pool_read(&wrong_window).is_err());
    let aligned = PoolRequest::Logs {
        schema_version: 1,
        chain_id: 42161,
        from_block: 112,
        to_block: 119,
    };
    assert_eq!(f.rpc.pool_read(&aligned).unwrap()["logs"], json!([]));
}

#[test]
fn pool_checkpoint_is_exact_finalized_block_with_bounded_active_tree_count_and_root() {
    let f = fixture();
    let request = PoolRequest::Checkpoint {
        schema_version: 1,
        chain_id: 42161,
        number: 119,
    };
    let checkpoint = f.rpc.pool_read(&request).unwrap();
    assert_eq!(checkpoint["kind"], "checkpoint");
    assert_eq!(checkpoint["block"]["number"], 119);
    assert_eq!(checkpoint["tree_number"], 1);
    assert_eq!(checkpoint["next_leaf_index"], 2);
    assert_eq!(checkpoint["merkle_root"], f.env.state_root);
    assert!(f
        .rpc
        .pool_read(&PoolRequest::Checkpoint {
            schema_version: 1,
            chain_id: 42161,
            number: 121
        })
        .is_err());
    f.state.lock().unwrap().next_leaf_index = 65537;
    assert!(f.rpc.pool_read(&request).is_err());
}

fn reviewed_history_fixture() -> Vec<HistoricalDeployment> {
    let p = policy();
    // Synthetic code epochs exercise an absent future implementation, changing
    // pool/token bytecode and proxy storage. These are never deployment pins.
    [(110, 47, 2, 92), (118, 46, 1, 91)]
        .into_iter()
        .map(
            |(until_block, address, code, storage)| HistoricalDeployment {
                until_block,
                pool_code_keccak256: keccak(&[0x60, code, 0x60, code]),
                token_code_keccak256: keccak(&[0x60, code, 0x60, code]),
                token_not_yet_deployed: false,
                implementation_code_pins: vec![CodePin {
                    address: format!("0x{}", hex::encode([address; 20])),
                    keccak256: keccak(&[0x60, code, 0x60, code]),
                }],
                proxy_storage_pins: vec![StoragePin {
                    address: p.domain.pool_contract.clone(),
                    slot: hash(91),
                    value: hash(storage),
                }],
            },
        )
        .collect()
}

fn checkpoint_request(number: u64) -> PoolRequest {
    PoolRequest::Checkpoint {
        schema_version: 1,
        chain_id: 42161,
        number,
    }
}

#[test]
fn historical_deployment_epochs_support_exact_boundaries_without_future_code() {
    let f = fixture_with_history(reviewed_history_fixture());
    for number in [100, 109, 110, 117, 118, 120] {
        let response = f.rpc.pool_read(&checkpoint_request(number)).unwrap();
        assert_eq!(response["block"]["number"], number);
    }
    // The unchanged current profile is also used for actual admission/quotes.
    f.rpc.verify_admission(&f.env, &f.verified).unwrap();
    f.rpc.quote(&f.env, &f.verified).unwrap();
    assert!(f.rpc.pool_read(&checkpoint_request(99)).is_err());
}

#[test]
fn historical_token_absence_is_an_explicit_verified_prefix_not_a_current_exception() {
    let mut history = reviewed_history_fixture();
    history[0].token_not_yet_deployed = true;
    history[0].token_code_keccak256 = keccak(&[]);
    let f = fixture_with_history(history);
    f.state.lock().unwrap().token_deployment_block = Some(110);
    for number in [100, 109, 110, 117, 118, 120] {
        assert_eq!(
            f.rpc.pool_read(&checkpoint_request(number)).unwrap()["block"]["number"],
            number
        );
    }
    f.rpc.verify_admission(&f.env, &f.verified).unwrap();
    f.rpc.quote(&f.env, &f.verified).unwrap();
    f.state.lock().unwrap().head = 105;
    // The historical absence evidence is readable, but cannot fund or admit
    // an operation even when the two providers agree on a fresh old head.
    f.rpc.pool_read(&checkpoint_request(105)).unwrap();
    assert!(f.rpc.verify_admission(&f.env, &f.verified).is_err());
    assert!(f.rpc.quote(&f.env, &f.verified).is_err());
    assert!(f.rpc.opening_treasury().is_err());
    assert!(f.rpc.verify_pending_exposure("1").is_err());
    {
        let mut state = f.state.lock().unwrap();
        state.head = 120;
        state.token_deployment_block = Some(121);
    }
    // Once the current profile is selected, missing token code is always an
    // error. An older reviewed absence interval cannot authorize it.
    assert!(f.rpc.pool_read(&checkpoint_request(120)).is_err());
    assert!(f.rpc.verify_admission(&f.env, &f.verified).is_err());
    assert!(f.rpc.quote(&f.env, &f.verified).is_err());
    assert!(f.rpc.opening_treasury().is_err());
}

#[test]
fn historical_token_absence_rejects_provider_disagreement_unexpected_code_and_bad_pool() {
    let mut history = reviewed_history_fixture();
    history[0].token_not_yet_deployed = true;
    history[0].token_code_keccak256 = keccak(&[]);
    let f = fixture_with_history(history);
    f.state.lock().unwrap().token_deployment_block = Some(110);
    f.rpc.pool_read(&checkpoint_request(105)).unwrap();
    f.state.lock().unwrap().token_absence_disagreement = true;
    assert!(f.rpc.pool_read(&checkpoint_request(105)).is_err());
    {
        let mut state = f.state.lock().unwrap();
        state.token_absence_disagreement = false;
        state.token_deployment_block = Some(100);
    }
    assert!(f.rpc.pool_read(&checkpoint_request(105)).is_err());
    {
        let mut state = f.state.lock().unwrap();
        state.token_deployment_block = Some(110);
    }
    let mut bad_pool = f.rpc_config.clone();
    bad_pool.historical_deployments[0].pool_code_keccak256 = keccak(&[0x60, 9]);
    let rpc = ArbitrumRpc::local_fixture(bad_pool, f.policy.clone()).unwrap();
    assert!(rpc.pool_read(&checkpoint_request(105)).is_err());
}

#[test]
fn historical_token_absence_configuration_cannot_skip_pins_or_reappear_after_deployment() {
    let mut history = reviewed_history_fixture();
    // A prefix may span pool implementation upgrades before the token exists.
    for epoch in &mut history {
        epoch.token_not_yet_deployed = true;
        epoch.token_code_keccak256 = keccak(&[]);
    }
    let f = fixture_with_history(history);
    f.state.lock().unwrap().token_deployment_block = Some(118);
    for number in [100, 109, 110, 117, 118, 120] {
        f.rpc.pool_read(&checkpoint_request(number)).unwrap();
    }
    for defect in [
        "wrong_absence_hash",
        "unmarked_empty_hash",
        "present_then_absent",
        "absent_current",
    ] {
        let mut config = f.rpc_config.clone();
        match defect {
            "wrong_absence_hash" => config.historical_deployments[0].token_code_keccak256 = hash(1),
            "unmarked_empty_hash" => {
                config.historical_deployments[0].token_not_yet_deployed = false
            }
            "present_then_absent" => {
                config.historical_deployments[0].token_not_yet_deployed = false;
                config.historical_deployments[0].token_code_keccak256 = keccak(&[0x60, 2, 0x60, 2]);
            }
            "absent_current" => config.token_code_keccak256 = keccak(&[]),
            _ => unreachable!(),
        }
        assert!(
            ArbitrumRpc::local_fixture(config, f.policy.clone()).is_err(),
            "accepted {defect}"
        );
    }
    let mut value = serde_json::to_value(&f.rpc_config).unwrap();
    value["token_not_yet_deployed"] = json!(true);
    assert!(serde_json::from_value::<RpcConfig>(value).is_err());
    // Existing complete profiles retain strict token presence when the new
    // optional historical field is absent from older configuration files.
    let mut value = serde_json::to_value(reviewed_history_fixture()[0].clone()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("token_not_yet_deployed");
    assert!(
        !serde_json::from_value::<HistoricalDeployment>(value)
            .unwrap()
            .token_not_yet_deployed
    );
}

#[test]
fn historical_deployment_missing_or_wrong_epochs_never_fall_back_to_current_pins() {
    let f = fixture_with_history(reviewed_history_fixture());
    for defect in [
        "missing",
        "wrong_boundary",
        "wrong_implementation",
        "wrong_storage",
        "wrong_pool",
        "wrong_token",
    ] {
        let mut config = f.rpc_config.clone();
        match defect {
            "missing" => config.historical_deployments.clear(),
            "wrong_boundary" => config.historical_deployments[0].until_block = 111,
            "wrong_implementation" => {
                config.historical_deployments[0].implementation_code_pins =
                    config.implementation_code_pins.clone()
            }
            "wrong_storage" => {
                config.historical_deployments[0].proxy_storage_pins[0].value = hash(90)
            }
            "wrong_pool" => {
                config.historical_deployments[0].pool_code_keccak256 =
                    config.pool_code_keccak256.clone()
            }
            "wrong_token" => {
                config.historical_deployments[0].token_code_keccak256 =
                    config.token_code_keccak256.clone()
            }
            _ => unreachable!(),
        }
        let rpc = ArbitrumRpc::local_fixture(config, f.policy.clone()).unwrap();
        let number = if defect == "wrong_boundary" { 110 } else { 105 };
        assert!(
            rpc.pool_read(&checkpoint_request(number)).is_err(),
            "accepted {defect}"
        );
    }
}

#[test]
fn historical_profiles_cannot_authorize_current_admission_or_release_checks() {
    let f = fixture_with_history(reviewed_history_fixture());
    f.state.lock().unwrap().head = 115;
    // Read-only historical evidence remains qualified at this height.
    assert_eq!(
        f.rpc.pool_read(&checkpoint_request(115)).unwrap()["block"]["number"],
        115
    );
    // Even a fresh agreeing head is not current until the configured boundary.
    assert!(f.rpc.verify_admission(&f.env, &f.verified).is_err());
    assert!(f.rpc.quote(&f.env, &f.verified).is_err());
    assert!(f.rpc.opening_treasury().is_err());
    assert!(f.rpc.verify_pending_exposure("1").is_err());
    f.state.lock().unwrap().head = 118;
    f.rpc.verify_admission(&f.env, &f.verified).unwrap();
    f.rpc.verify_pending_exposure("1").unwrap();
}

#[test]
fn historical_configuration_is_bounded_ordered_complete_and_strict() {
    let f = fixture_with_history(reviewed_history_fixture());
    for defect in [
        "zero_width",
        "before_deployment",
        "unordered",
        "missing_code",
        "missing_storage",
        "duplicate_code",
        "duplicate_slot",
        "bad_hash",
        "too_many",
    ] {
        let mut config = f.rpc_config.clone();
        match defect {
            "zero_width" => config.historical_deployments[1].until_block = 110,
            "before_deployment" => config.historical_deployments[0].until_block = 99,
            "unordered" => config.historical_deployments.reverse(),
            "missing_code" => config.historical_deployments[0]
                .implementation_code_pins
                .clear(),
            "missing_storage" => config.historical_deployments[0].proxy_storage_pins.clear(),
            "duplicate_code" => {
                let pin = config.historical_deployments[0].implementation_code_pins[0].clone();
                config.historical_deployments[0]
                    .implementation_code_pins
                    .push(pin);
            }
            "duplicate_slot" => {
                let pin = config.historical_deployments[0].proxy_storage_pins[0].clone();
                config.historical_deployments[0]
                    .proxy_storage_pins
                    .push(pin);
            }
            "bad_hash" => config.historical_deployments[0].token_code_keccak256 = "0x00".into(),
            "too_many" => {
                let epoch = config.historical_deployments[0].clone();
                config.historical_deployments = (101..=165)
                    .map(|until_block| HistoricalDeployment {
                        until_block,
                        ..epoch.clone()
                    })
                    .collect();
            }
            _ => unreachable!(),
        }
        assert!(
            ArbitrumRpc::local_fixture(config, f.policy.clone()).is_err(),
            "accepted {defect}"
        );
    }
    let mut value = serde_json::to_value(&f.rpc_config).unwrap();
    value["historical_deployments"][0]["from_block"] = json!(100);
    assert!(serde_json::from_value::<RpcConfig>(value).is_err());
    let mut value = serde_json::to_value(&f.rpc_config).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("historical_deployments");
    assert!(serde_json::from_value::<RpcConfig>(value)
        .unwrap()
        .historical_deployments
        .is_empty());
}

fn complete_block_fixture(f: &Fixture) -> Value {
    let transactions = vec![hash(61), hash(62)];
    let raw_log = |tx: usize, index: u64, address: &str, topics: Value, data: Value| {
        json!({"address":address,"topics":topics,"data":data,"blockHash":block_hash(122),
            "blockNumber":"0x7a","transactionHash":transactions[tx],
            "transactionIndex":quantity(tx as u64),"logIndex":quantity(index),"removed":false})
    };
    let mut logs = vec![raw_log(
        0,
        0,
        "0x1111111111111111111111111111111111111111",
        json!([]),
        json!("0x"),
    )];
    for (index, log) in abi()["logs"].as_array().unwrap().iter().enumerate() {
        logs.push(raw_log(
            0,
            index as u64 + 1,
            &f.policy.domain.pool_contract,
            log["topics"].clone(),
            log["data"].clone(),
        ));
    }
    let second = raw_log(
        1,
        logs.len() as u64,
        &f.policy.domain.pool_contract,
        json!([hash(63)]),
        json!("0x"),
    );
    let receipts: Vec<_> = [logs, vec![second]]
        .into_iter()
        .enumerate()
        .map(|(index, logs)| {
            json!({"transactionHash":transactions[index],"transactionIndex":quantity(index as u64),
            "blockHash":block_hash(122),"blockNumber":"0x7a","status":"0x1", "gasUsed":"0xc350",
            "effectiveGasPrice":"0x2","logs":logs})
        })
        .collect();
    let receipts = json!(receipts);
    let mut state = f.state.lock().unwrap();
    state.head = 124;
    state.block_transactions.insert(122, transactions);
    state.block_receipts.insert(122, receipts.clone());
    receipts
}

fn block_logs_request() -> PoolRequest {
    PoolRequest::BlockLogs {
        schema_version: 1,
        chain_id: 42161,
        number: 122,
    }
}

#[test]
fn pool_block_logs_checks_complete_block_before_fixed_pool_filtering() {
    let f = fixture();
    complete_block_fixture(&f);
    // The independently fetched whole-block receipts contain events omitted by
    // this fixture's eth_getLogs path. A caller can detect that mismatch.
    assert_eq!(
        f.rpc
            .pool_read(&PoolRequest::Logs {
                schema_version: 1,
                chain_id: 42161,
                from_block: 122,
                to_block: 122,
            })
            .unwrap()["logs"],
        json!([])
    );
    let result = f.rpc.pool_read(&block_logs_request()).unwrap();
    assert_eq!(result["block"]["number"], 122);
    assert_eq!(result["block"]["hash"], block_hash(122));
    assert_eq!(result["kind"], "block_logs");
    let logs = result["logs"].as_array().unwrap();
    assert_eq!(logs.len(), abi()["logs"].as_array().unwrap().len());
    assert_eq!(logs[0]["logIndex"], "0x1");
    assert_eq!(logs[1]["logIndex"], "0x2");
    assert!(logs
        .iter()
        .all(|log| log["address"] == f.policy.domain.pool_contract));
    assert!(serde_json::from_value::<PoolRequest>(json!({"schema_version":1,"chain_id":42161,"kind":"block_logs","number":122,"address":f.policy.domain.pool_contract})).is_err());
    assert!(f
        .rpc
        .pool_read(&PoolRequest::BlockLogs {
            schema_version: 1,
            chain_id: 42161,
            number: 125
        })
        .is_err());
}

#[test]
fn pool_block_logs_rejects_omitted_duplicate_and_misindexed_receipts() {
    let f = fixture();
    let good = complete_block_fixture(&f);
    for defect in [
        "omitted",
        "duplicate",
        "wrong_index",
        "wrong_hash",
        "wrong_block",
        "unsupported",
    ] {
        let mut bad = good.clone();
        match defect {
            "omitted" => {
                bad.as_array_mut().unwrap().pop();
            }
            "duplicate" => bad[1] = bad[0].clone(),
            "wrong_index" => bad[1]["transactionIndex"] = json!("0x0"),
            "wrong_hash" => bad[1]["transactionHash"] = json!(hash(64)),
            "wrong_block" => bad[1]["blockHash"] = json!(hash(65)),
            "unsupported" => bad = Value::Null,
            _ => unreachable!(),
        }
        f.state.lock().unwrap().block_receipts.insert(122, bad);
        assert!(
            f.rpc.pool_read(&block_logs_request()).is_err(),
            "accepted {defect}"
        );
    }
}

#[test]
fn pool_block_logs_rejects_log_gaps_wrong_context_and_provider_disagreement() {
    let f = fixture();
    let good = complete_block_fixture(&f);
    for defect in [
        "log_gap",
        "wrong_transaction",
        "wrong_block",
        "removed",
        "reverted_logs",
    ] {
        let mut bad = good.clone();
        match defect {
            "log_gap" => bad[0]["logs"].as_array_mut().unwrap().remove(0),
            "wrong_transaction" => {
                bad[0]["logs"][1]["transactionHash"] = json!(hash(62));
                Value::Null
            }
            "wrong_block" => {
                bad[0]["logs"][1]["blockNumber"] = json!("0x79");
                Value::Null
            }
            "removed" => {
                bad[0]["logs"][1]["removed"] = json!(true);
                Value::Null
            }
            "reverted_logs" => {
                bad[0]["status"] = json!("0x0");
                Value::Null
            }
            _ => unreachable!(),
        };
        f.state.lock().unwrap().block_receipts.insert(122, bad);
        assert!(
            f.rpc.pool_read(&block_logs_request()).is_err(),
            "accepted {defect}"
        );
    }
    let mut other = good.clone();
    other[1]["effectiveGasPrice"] = json!("0x3");
    {
        let mut state = f.state.lock().unwrap();
        state.block_receipts.insert(122, good);
        state.block_receipts_secondary = Some(other);
    }
    assert!(f.rpc.pool_read(&block_logs_request()).is_err());
}

#[test]
fn pool_block_logs_rechecks_canonical_block_after_complete_receipts() {
    let f = fixture();
    complete_block_fixture(&f);
    f.state.lock().unwrap().reorg_after_block_receipts = true;
    assert!(f.rpc.pool_read(&block_logs_request()).is_err());
    assert!(f.state.lock().unwrap().reorg);
}

#[test]
fn pool_block_logs_bounds_transaction_lists_and_accepts_verified_empty_block() {
    let f = fixture();
    complete_block_fixture(&f);
    {
        let mut state = f.state.lock().unwrap();
        state.block_transactions.insert(
            122,
            (0..4097)
                .map(|n| keccak(format!("public-fixture-{n}").as_bytes()))
                .collect(),
        );
        state.block_receipts.insert(122, json!([]));
    }
    assert!(f.rpc.pool_read(&block_logs_request()).is_err());
    f.state
        .lock()
        .unwrap()
        .block_transactions
        .insert(122, vec![]);
    assert_eq!(
        f.rpc.pool_read(&block_logs_request()).unwrap()["logs"],
        json!([])
    );
}

#[test]
fn live_treasury_check_covers_total_exposure_and_blocks_release_after_balance_loss() {
    let f = fixture();
    f.state.lock().unwrap().balance = 1_500_000;
    f.rpc.verify_pending_exposure("1000000").unwrap();
    assert_eq!(
        f.rpc.verify_pending_exposure("2000000"),
        Err(Error::BudgetExceeded)
    );
    let id = &f.env.operation_digest;
    f.service.admit(id, &encode(&f.env).unwrap(), 1500).unwrap();
    f.state.lock().unwrap().balance = 500_000;
    assert_eq!(f.worker.process_once(id), Err(Error::BudgetExceeded));
    assert!(f.state.lock().unwrap().sent.is_empty());
    assert_eq!(f.service.summary().unwrap().reserved_native_wei, "1000000");
}

#[test]
fn worker_audits_previously_finalized_receipts_and_halts_on_canonical_contradiction() {
    let f = fixture();
    let id = &f.env.operation_digest;
    f.service.admit(id, &encode(&f.env).unwrap(), 1500).unwrap();
    f.worker.process_once(id).unwrap();
    mine(&f, false);
    assert_eq!(
        f.worker.process_once(id).unwrap().phase,
        JobPhase::Finalized
    );
    f.state.lock().unwrap().reorg = true;
    assert_eq!(f.worker.process_batch(32).unwrap(), 1);
    assert_eq!(f.service.get_job(id).unwrap().phase, JobPhase::Incident);
    let summary = f.service.summary().unwrap();
    assert!(summary.halted);
    assert_eq!(summary.reserved_native_wei, "1000000");
    assert_eq!(summary.earned_fee_units, "0");
    assert_eq!(summary.finalized_native_charge_wei, "0");
    assert_eq!(f.worker.process_batch(32), Err(Error::Halted));
}

#[test]
fn one_slot_batch_does_not_starve_finalized_audit_when_live_job_exists() {
    let f = fixture();
    let id = &f.env.operation_digest;
    f.service.admit(id, &encode(&f.env).unwrap(), 1500).unwrap();
    f.worker.process_once(id).unwrap();
    mine(&f, false);
    assert_eq!(
        f.worker.process_once(id).unwrap().phase,
        JobPhase::Finalized
    );
    // This cfg(test)-only proof adapter permits an independently reserved live
    // job; the first one-slot batch must audit the terminal job before it.
    let mut other = f.env.clone();
    other.nullifiers = vec![hash(71), hash(72)];
    other.operation_digest = envelope_digest(&other).unwrap();
    f.service
        .admit(&other.operation_digest, &encode(&other).unwrap(), 1500)
        .unwrap();
    f.state.lock().unwrap().reorg = true;
    assert_eq!(f.worker.process_batch(1).unwrap(), 1);
    assert_eq!(f.service.get_job(id).unwrap().phase, JobPhase::Incident);
    assert_eq!(
        f.service.get_job(&other.operation_digest).unwrap().phase,
        JobPhase::Admitted
    );
    assert!(f.service.summary().unwrap().halted);
}

#[test]
fn finalized_revert_allows_exact_proof_retry_with_fresh_nonce_and_cumulative_native_cost() {
    let f = fixture();
    let id = &f.env.operation_digest;
    let body = encode(&f.env).unwrap();
    f.service.admit(id, &body, 1500).unwrap();
    f.worker.process_once(id).unwrap();
    mine(&f, true);
    assert_eq!(
        f.worker.process_once(id).unwrap().phase,
        JobPhase::FinalizedRevert
    );
    // The old schedule cannot admit a NEW operation, but its existing obligation
    // and exact fee/cumulative cost cap remain unchanged for this explicit retry.
    let retry = f.service.admit(id, &body, 10_000).unwrap();
    assert_eq!(retry.phase, JobPhase::Admitted);
    assert_eq!(retry.attempt_number, 2);
    assert_eq!(retry.reserved_native_wei, "900000");
    assert_eq!(retry.finalized_native_charge_wei, "100000");
    assert_eq!(f.service.summary().unwrap().jobs, 1);
    let pending = f.worker.process_once(id).unwrap();
    assert_eq!(pending.nonce, Some(1));
    assert_eq!(pending.attempt_number, 2);
    assert_eq!(pending.phase, JobPhase::Pending);
    let raws = f.state.lock().unwrap().sent.clone();
    assert_eq!(raws.len(), 2);
    assert_ne!(raws[0], raws[1]);
    for raw in &raws {
        crate::transaction::inspect_outer(raw, &f.policy, &f.verified).unwrap();
    }
    mine(&f, false);
    let final_view = f.worker.process_once(id).unwrap();
    assert_eq!(final_view.phase, JobPhase::Finalized);
    assert_eq!(final_view.finalized_native_charge_wei, "200000");
    assert_eq!(final_view.earned_fee_units, "20000");
    assert_eq!(final_view.reserved_native_wei, "0");
    // Earlier attempts remain subject to canonical auditing after success.
    f.state.lock().unwrap().reorg = true;
    assert_eq!(f.worker.process_once(id).unwrap().phase, JobPhase::Incident);
    assert!(f.service.summary().unwrap().halted);
}

#[test]
fn retry_requires_finalized_revert_and_fresh_unspent_inputs_and_has_three_attempt_limit() {
    let f = fixture();
    let id = &f.env.operation_digest;
    let body = encode(&f.env).unwrap();
    f.service.admit(id, &body, 1500).unwrap();
    f.worker.process_once(id).unwrap();
    assert_eq!(
        f.service.retry_finalized_revert(id, 1501),
        Err(Error::InvalidTransition)
    );
    assert_eq!(f.service.get_job(id).unwrap().attempt_number, 1);
    for attempt in 1..=3 {
        mine(&f, true);
        let view = f.worker.process_once(id).unwrap();
        assert_eq!(view.attempt_number, attempt);
        if attempt < 3 {
            assert_eq!(view.phase, JobPhase::FinalizedRevert);
            f.state.lock().unwrap().spent = true;
            assert_eq!(f.service.admit(id, &body, 2500), Err(Error::Conflict));
            assert_eq!(f.service.get_job(id).unwrap().attempt_number, attempt);
            f.state.lock().unwrap().spent = false;
            f.service.admit(id, &body, 2500).unwrap();
            f.worker.process_once(id).unwrap();
        } else {
            assert_eq!(view.phase, JobPhase::RetryExhausted);
            assert_eq!(view.finalized_native_charge_wei, "300000");
            assert_eq!(view.earned_fee_units, "0");
            assert_eq!(
                f.service.retry_finalized_revert(id, 2500),
                Err(Error::InvalidTransition)
            );
            assert_eq!(f.service.admit(id, &body, 2500).unwrap(), view);
        }
    }
    assert_eq!(f.state.lock().unwrap().sent.len(), 3);
    assert_eq!(f.service.summary().unwrap().jobs, 1);
}

#[test]
fn public_pool_receipt_can_reconcile_another_broadcaster_without_charging_operator() {
    let f = fixture();
    let id = &f.env.operation_digest;
    f.service.admit(id, &encode(&f.env).unwrap(), 1500).unwrap();
    f.worker.process_once(id).unwrap();
    mine(&f, false);
    f.state.lock().unwrap().external_sender = true;
    let tx = f.state.lock().unwrap().transaction_hash().unwrap();
    let response = f
        .rpc
        .pool_read(&PoolRequest::Receipt {
            schema_version: 1,
            chain_id: 42161,
            transaction_hash: tx.clone(),
        })
        .unwrap();
    assert_eq!(response["transaction_input"], f.env.call_data);
    assert_eq!(response["receipt"]["status"], "0x1");
    assert!(response["receipt"].get("from").is_none());
    assert!(f
        .rpc
        .operation_receipt(id, &f.env, &f.verified, &tx)
        .is_err());
    assert_eq!(f.service.summary().unwrap().earned_fee_units, "0");
}

#[test]
fn elapsed_unknown_attempt_only_reconciles_until_explicit_identical_resubmission() {
    let f = fixture();
    let id = &f.env.operation_digest;
    let body = encode(&f.env).unwrap();
    f.service.admit(id, &body, 1500).unwrap();
    let raw = crate::transaction::sign_fixture(&f.policy, &f.verified, 0, 3);
    f.service.record_signed_outer(id, &raw).unwrap();
    // Test-only old release timestamp models an interrupted network call long
    // before this worker restarted, without sleeping or changing real time.
    {
        let connection = f.service.lock().unwrap();
        append(&connection, id, &Event::Released { at: 1 }).unwrap();
    }
    assert_eq!(f.worker.process_once(id).unwrap().phase, JobPhase::Unknown);
    assert!(f.state.lock().unwrap().sent.is_empty());
    let requested = f.service.admit(id, &body, 2500).unwrap();
    assert_eq!(requested.attempt_number, 1);
    assert_eq!(requested.phase, JobPhase::Unknown);
    let pending = f.worker.process_once(id).unwrap();
    assert_eq!(pending.phase, JobPhase::Pending);
    assert_eq!(pending.nonce, Some(0));
    assert_eq!(pending.attempt_number, 1);
    assert_eq!(f.state.lock().unwrap().sent, vec![raw]);
}
