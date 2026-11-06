//! Native operator signer: no public arbitrary-message or arbitrary-call method.
//! GasService obtains approved fields from its journal and commits the signature
//! before any broadcast release. A signer object never enables release gates.
use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use zeroize::Zeroizing;

use crate::model::{decimal, AdmissionPolicy, VerifiedOperation};
use crate::secrets::SecretSource;
use crate::transaction::{inspect_outer, sign_approved, SignedOuter};
use crate::{Error, Result};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GasQuote {
    pub nonce: u64,
    pub max_priority_fee_per_gas_wei: String,
    pub max_fee_per_gas_wei: String,
}

pub struct NativeTreasurySigner {
    key: SigningKey,
    address: String,
}

impl NativeTreasurySigner {
    pub fn from_source(source: &SecretSource) -> Result<Self> {
        Self::from_key(source.read()?)
    }

    pub(crate) fn from_key(secret: Zeroizing<[u8; 32]>) -> Result<Self> {
        let key = SigningKey::from_slice(secret.as_ref()).map_err(|_| Error::SecretInput)?;
        let bytes = key.verifying_key().to_sec1_point(false);
        let address = format!(
            "0x{}",
            hex::encode(&Keccak256::digest(&bytes.as_bytes()[1..])[12..])
        );
        Ok(Self { key, address })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub(crate) fn sign(
        &self,
        policy: &AdmissionPolicy,
        verified: &VerifiedOperation,
        quote: &GasQuote,
    ) -> Result<SignedOuter> {
        if self.address != policy.broadcaster_address {
            return Err(Error::InvalidOuterTransaction);
        }
        let raw = sign_approved(
            &self.key,
            policy,
            verified,
            quote.nonce,
            decimal(&quote.max_priority_fee_per_gas_wei)?,
            decimal(&quote.max_fee_per_gas_wei)?,
        )?;
        inspect_outer(&raw, policy, verified)
    }
}
