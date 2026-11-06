//! Operator runtime configuration and orchestration. Startup can serve genuine
//! encrypted diagnostics while production economics remain release-gated.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::fee_schedule::SignedFeeSchedule;
use crate::gateway::ObliviousGateway;
use crate::proof_process::{NodeProofVerifier, NodeVerifierConfig};
use crate::rpc::{ArbitrumRpc, RpcConfig};
use crate::secrets::SecretSource;
use crate::signer::NativeTreasurySigner;
use crate::worker::OperatorWorker;
use crate::{
    AdmissionPolicy, ChainEvidenceVerifier, Error, GasService, ProofVerifier, Result,
    UnavailableChainVerifier, UnavailableProofVerifier,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretReference {
    File { path: PathBuf },
    Environment { name: String },
    Stdin,
}
impl SecretReference {
    fn input(&self) -> Result<SecretSource> {
        match self {
            Self::File { path } => {
                if !path.is_absolute() {
                    return Err(Error::InvalidPolicy);
                }
                Ok(SecretSource::File(path.clone()))
            }
            Self::Environment { name } => {
                if !name.starts_with("WALLET_GAS_")
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                {
                    return Err(Error::InvalidPolicy);
                }
                Ok(SecretSource::Environment(name.clone()))
            }
            Self::Stdin => Ok(SecretSource::Stdin),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub schema_version: u8,
    pub listen: SocketAddr,
    pub authority: String,
    pub gateway_key_id: u8,
    pub gateway_secret: SecretReference,
    pub journal: PathBuf,
    pub policy: AdmissionPolicy,
    pub fee_schedule: SignedFeeSchedule,
    pub pinned_fee_public_key: String,
    pub proof_verifier: Option<NodeVerifierConfig>,
    pub rpc: Option<RpcConfig>,
    pub treasury_secret: Option<SecretReference>,
}

impl RuntimeConfig {
    pub fn read(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).map_err(|_| Error::InvalidPolicy)?;
        let mut text = String::new();
        file.take(262_145)
            .read_to_string(&mut text)
            .map_err(|_| Error::InvalidPolicy)?;
        if text.len() > 262_144 {
            return Err(Error::InvalidPolicy);
        }
        let config: Self = crate::model::decode(&text)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || !self.listen.ip().is_loopback()
            || !self.journal.is_absolute()
            || matches!(self.gateway_secret, SecretReference::Stdin)
                && matches!(self.treasury_secret, Some(SecretReference::Stdin))
        {
            return Err(Error::InvalidPolicy);
        }
        self.policy.validate()?;
        crate::gateway::validate_authority(&self.authority)?;
        if let Some(config) = &self.rpc {
            ArbitrumRpc::validate_configuration(config, &self.policy)?;
        }
        if let Some(config) = &self.proof_verifier {
            config.validate_shape()?;
            if config.additional_native_fee_cap_wei != "0" {
                return Err(Error::InvalidPolicy);
            }
        }
        self.gateway_secret.input()?;
        if let Some(secret) = &self.treasury_secret {
            secret.input()?;
        }
        let public = self.fee_key()?;
        self.fee_schedule.verify(&public)?;
        // The signed schedule may rotate the ID, fee and epoch in the same DB.
        let mut next = self.policy.clone();
        next.schedule_id = self.fee_schedule.schedule.schedule_id.clone();
        next.fee_units = self.fee_schedule.schedule.fee_units.clone();
        next.valid_from = self.fee_schedule.schedule.valid_from;
        next.valid_until = self.fee_schedule.schedule.valid_until;
        self.fee_schedule.verify_policy(&public, &next)
    }
    fn fee_key(&self) -> Result<[u8; 32]> {
        crate::model::bounded_hex(&self.pinned_fee_public_key, 32)?
            .try_into()
            .map_err(|_| Error::InvalidPolicy)
    }

    pub fn public_gateway_configuration(&self) -> Result<serde_json::Value> {
        self.validate()?;
        let key = ObliviousGateway::public_key_from_secret(
            self.gateway_key_id,
            &self.gateway_secret.input()?,
        )?;
        Ok(
            serde_json::json!({"schema_version":1,"encoded_key":format!("0x{}",hex::encode(&key)),
            "sha256":format!("0x{}",hex::encode(Sha256::digest(&key))),"production_sponsorship_available":false}),
        )
    }
}

pub struct OperatorRuntime {
    pub gateway: Arc<ObliviousGateway>,
    worker: Option<Arc<OperatorWorker>>,
    listen: SocketAddr,
}

impl OperatorRuntime {
    pub fn build(config: RuntimeConfig) -> Result<Self> {
        config.validate()?;
        let rpc = config
            .rpc
            .clone()
            .map(|value| ArbitrumRpc::new(value, config.policy.clone()).map(Arc::new))
            .transpose()?;
        let proof: Arc<dyn ProofVerifier> = match &config.proof_verifier {
            Some(value) => Arc::new(NodeProofVerifier::new(value.clone())?),
            None => Arc::new(UnavailableProofVerifier),
        };
        let chain: Arc<dyn ChainEvidenceVerifier> = match &rpc {
            Some(value) => value.clone(),
            None => Arc::new(UnavailableChainVerifier),
        };
        let service = Arc::new(GasService::open(
            &config.journal,
            config.policy.clone(),
            proof,
            chain,
        )?);
        let fee_key = config.fee_key()?;
        let mut gateway = ObliviousGateway::from_secret(
            config.authority,
            config.gateway_key_id,
            &config.gateway_secret.input()?,
            service.clone(),
            config.fee_schedule,
            fee_key,
        )?;
        if let Some(rpc) = &rpc {
            gateway = gateway.with_pool_rpc(rpc.clone());
        }
        // Do not even load a treasury private key while release requirements are
        // unmet. Possession of a key or successful RPC configuration is no bypass.
        let worker = if service.require_active().is_ok() {
            let secret = config
                .treasury_secret
                .as_ref()
                .ok_or(Error::IntegrationUnavailable)?
                .input()?;
            let signer = NativeTreasurySigner::from_source(&secret)?;
            let worker = Arc::new(OperatorWorker::new(
                service,
                rpc.ok_or(Error::IntegrationUnavailable)?,
                signer,
            )?);
            worker.initialize_treasury()?;
            Some(worker)
        } else {
            None
        };
        Ok(Self {
            gateway: Arc::new(gateway),
            worker,
            listen: config.listen,
        })
    }

    pub fn public_configuration(&self) -> Result<serde_json::Value> {
        let key = self.gateway.public_configuration()?;
        Ok(
            serde_json::json!({"schema_version":1,"encoded_key":format!("0x{}",hex::encode(&key)),
            "sha256":format!("0x{}",hex::encode(Sha256::digest(&key))),"production_sponsorship_available":false}),
        )
    }

    pub fn serve(self) -> Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(8)
            .enable_all()
            .build()
            .map_err(|_| Error::Gateway)?;
        runtime.block_on(async move {
            let listener=tokio::net::TcpListener::bind(self.listen).await.map_err(|_|Error::Gateway)?;
            let address=listener.local_addr().map_err(|_|Error::Gateway)?;
            // One startup record only; no request counters, addresses, request IDs
            // or errors are emitted for individual operations.
            println!("{}",serde_json::json!({"listen":address.to_string(),"ohttp_gateway":true,"production_sponsorship_available":false}));
            let (stop,receiver)=tokio::sync::watch::channel(false);
            if let Some(worker)=self.worker {
                let mut stopped=receiver.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            changed=stopped.changed()=>{if changed.is_err()||*stopped.borrow(){break;}}
                            _=tokio::time::sleep(std::time::Duration::from_secs(5))=>{
                                let worker=worker.clone(); let _=tokio::task::spawn_blocking(move||worker.process_batch(32)).await;
                            }
                        }
                    }
                });
            }
            let serve=self.gateway.serve(listener,receiver); tokio::pin!(serve);
            tokio::select! {
                result=&mut serve=>result,
                signal=tokio::signal::ctrl_c()=>{
                    signal.map_err(|_|Error::Gateway)?; let _=stop.send(true); serve.await
                }
            }
        })
    }
}
